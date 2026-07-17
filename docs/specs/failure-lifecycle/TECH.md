# Failure Lifecycle Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #681

## Existing Implementation Facts

- Extraction replay ranges exist and are user-driven:
  `remem pending list-extraction-ranges` / `retry-extraction-ranges`
  (#426/#660 landed the machinery; doctor reports "27 extraction replay
  ranges retryable" on the reference install).
- Doctor already surfaces raw failed counts (capture-liveness probe and
  pending-queue WARN, #374) but with no age dimension and no 7-day split for
  jobs.
- #365 fixed compression AI failures being mis-marked successful; failure
  marking is honest today, and nothing consumes the failures afterward.
- Four failure-bearing surfaces are currently visible to status/doctor:
  `pending_observations` (legacy extraction queue, including
  `status='failed'`), `extraction_tasks` (2919 failed on the reference
  install), `extraction_replay_ranges` (27 retryable ranges on the reference
  install), and the background job queue (`jobs`, 2470 failed). All
  accumulate without a lifecycle split.

## Design

### 1. Failure taxonomy

New lifecycle columns on all four failure-bearing surfaces
(`pending_observations`, `extraction_tasks`, `extraction_replay_ranges`,
`jobs`):

- `failure_class TEXT` (`transient` | `permanent`);
- `failed_at_epoch INTEGER NULL`;
- `archived_at_epoch INTEGER NULL`.

Retry attempt counts reuse the table-native counters:

- `pending_observations.attempt_count`;
- `extraction_tasks.attempts`;
- `extraction_replay_ranges.attempts`;
- `jobs.attempt_count`.

When new code marks a row failed it must set `failed_at_epoch` at the same
time as status/state, and subsequent archiving or cleanup bookkeeping must
not rewrite it. For back-classified historical rows, `failed_at_epoch =
COALESCE(NULLIF(updated_at_epoch, 0), created_at_epoch)` so retry backoff and
retention ages are deterministic.

Classification maps existing error strings at failure-marking time:

- transient: AI timeout/rate-limit/5xx, HTTP transport errors, DB lock/busy,
  worker global-timeout interruption;
- permanent: schema/vocabulary mismatch, malformed payload, unsupported
  version markers, missing evidence rows;
- unknown strings default to `transient` (conservative: bounded retries, not
  premature burial).

The mapping table lives in one module with unit tests per pattern, so new
error strings get classified in one place.

### 2. Bounded auto-recovery

Worker loop extension (no new daemon): once per cycle, pick up to N
transient failures whose backoff window elapsed
(`next_retry = failed_at_epoch + base * 2^attempts`, attempts capped at
`MAX_FAILURE_RETRIES`, default 3). Every attempt logs class, attempt count,
source surface, and outcome. On cap exhaustion the row is marked exhausted
(attempts = cap) and becomes eligible for archiving. Permanent-class rows
are archive-eligible immediately.

Recovery paths are surface-specific. Any retry/requeue path that targets an
archived row must either clear `archived_at_epoch` in the same transaction
before making work pending again, or create a fresh retry row linked to the
archived source; no pending work may retain an archived marker.

- `pending_observations`: no automatic retry in v1 because the current runtime
  no longer has a production worker consumer for this legacy queue. Failed
  legacy rows are classified, reported, and archived so they stop polluting
  headline counts, while manual inspection remains available. If a production
  consumer is reintroduced later, this spec must be updated before automatic
  pending-observation retry is enabled.
- `extraction_replay_ranges`: invoke the existing
  `retry_extraction_replay_ranges` machinery for retryable ranges.
- `extraction_tasks` with a replay range: route through that range.
- `extraction_tasks` without a replay range: requeue the original task
  directly by setting `status='pending'`, clearing lease fields, and setting
  `next_retry_epoch`; no-range transient failures therefore have an explicit
  recovery path instead of staying actionable forever.
- `jobs`: exclude retired legacy Summary rows from candidate selection before
  generic recovery. The transaction-scoped per-row classifier must also check
  Summary before active-identity lookup and return an explicit retired/skipped
  result for defensive direct input. In both paths, preserve every persisted
  field byte/value; do not set permanent, change retry time, append a marker,
  execute the job, or increment `requeued`/`coalesced` counters. For non-retired
  job types, re-enqueue the failed job by setting `state='pending'`, clearing
  lease fields, and setting `next_retry_epoch`. If the same active job identity
  already exists, keep that canonical work active and leave the source as
  `failed` with `failure_class='permanent'` and `next_retry_epoch=0`. Preserve
  the source's real `attempt_count`, error, timestamps, payload, and id; append
  only a bounded non-secret canonical marker to `last_error`. When source
  `last_error` is NULL or empty, store the complete marker alone; only a
  non-empty error uses marker-space reservation, deterministic truncation, and
  append. The worker logs safe source/canonical ids and identity kind, never
  the original error text.
  This collision is a successful convergence result for the candidate, not a
  fabricated exhausted attempt or a successful completion of the source.
  Candidate ids are fully collected and the read statement released before
  per-row writes begin. Each row must acquire `IMMEDIATE` write ownership
  before re-reading source eligibility or looking up active identity; lookup
  before write ownership is forbidden. If requeue meets the active-identity
  UNIQUE constraint, only that declared identity conflict may trigger an exact
  canonical reread. A readable, still-active canonical row yields a structured
  coalesced result. A terminal, missing, or unreadable canonical row, a
  busy/locked failure, or any non-identity constraint error rolls back that
  source unchanged and propagates the error under `B-014`; recovery must not
  return a stale/non-persisted id or assume deduplication. File-backed,
  two-connection WAL barrier tests cover the identity race and unreadable
  canonical rollback while proving independently committed unrelated rows
  continue to make progress.

### 2.1 Job queue persisted truth and v069 lifecycle inputs

Lease-owned done, retry, exhausted, and permanent-failure transitions use the
current processing row, expected owner, and unexpired lease as a single
transactional authorization boundary. A missing-row result is an error with an
explicit `current=missing` diagnostic; no row is created, so shared stats gain
no processing or stuck entry. For an existing wrong-owner, reclaimed,
expired-lease, or otherwise ineligible row, rejection leaves every persisted
field unchanged. The worker must propagate either error and emit no done/retry
success signal. Shared stats reflect the existing row's actual persisted state:
if it is still `processing`, it remains counted there and becomes `stuck` after
its unchanged lease expires; an already reclaimed or non-processing row is
reported according to that state instead. No parallel in-memory success ledger
may override database truth.

The v069 job-queue migration contributes a separate failure-lifecycle input.
Each reconciled non-Summary active duplicate becomes `state='failed'`,
`failure_class='permanent'`, `archived_at_epoch=NULL`, and
`next_retry_epoch=0`, while retaining its real attempt count and bounded
existing error evidence plus the non-secret duplicate marker. It is an
actionable permanent failure in the shared stats/status/doctor source until the
existing retention step archives it; the migration must not raise its attempt
count to fabricate exhaustion. Late active Summary retirement is not such a
duplicate: v069 uses the exact v064 retirement marker so existing failure and
legacy-surface predicates continue to exclude it.

These v069 rows are not the historical v057 back-classification described in
section 5. The v057 upgrade deliberately initializes pre-existing failed rows
as exhausted to avoid a retry storm; v069 creates new conflict evidence and
must preserve each source row's actual attempt count. Neither rule changes the
retention, cleanup, or aggregate-history policy below.

### 3. Retention / archiving

A worker maintenance step transitions eligible rows to archived
(`archived_at_epoch` set) once they are older than
`failure_retention_days` (config, default 14). Archiving is a state
transition; rows and their error strings remain queryable until explicit
cleanup.

New table `failure_lifecycle_daily` preserves aggregate history before
cleanup can delete source rows: day bucket, surface
(`pending_observation` | `extraction_task` | `extraction_replay_range` |
`job`), failure_class, archived count, purged count, oldest/newest
`failed_at_epoch`, and last rollup epoch. Archiving and cleanup upsert this
table transactionally. Status/stats may still use live counts for current
rows, but historical totals after cleanup must come from this table, not
from live `COUNT(*)` queries.

`remem cleanup --archived-failures[=<days>]` (default horizon 90 days)
deletes archived rows older than the horizon, printing counts (explicit
purge only). Cleanup must be FK-safe for replay ranges:

- archived replay ranges are purged before their archived source/replay
  extraction tasks;
- if an extraction task is still referenced by a non-purged replay range,
  cleanup keeps the task and reports it as skipped;
- before deleting any replay range, cleanup clears nullable
  `extraction_tasks.replay_range_id` references for every task pointing at
  that range, including successful/non-archived replay tasks that are not
  purged in the same transaction; archived source/replay tasks are then
  deleted only after the range FK references are clear.

### 4. Reporting split

- Status/doctor headline counters exclude archived rows across all four
  surfaces. Actionable total = all non-archived failures plus retryable replay
  ranges; actionable 7d is a subcount for freshness/scanning context. The
  probe prints `actionable total`, `actionable 7d`, oldest actionable age,
  per-class counts, and `archived: <n>` as a secondary line.
- Severity: FAIL/WARN thresholds evaluate actionable total only; a store with
  thousands of archived and zero actionable-total failures reports ok. An
  8-14 day failure continues to affect severity until it archives.
- `remem status --json` adds `failures: {actionable_7d, actionable_total,
  transient, permanent, exhausted, archived, historical_archived,
  historical_purged, oldest_actionable_epoch}` per surface.

### 5. Back-classification migration

The schema migration back-fills existing failed rows on all four surfaces:
classify by error string where it matches the mapping; unmatched rows become
`transient`. All pre-existing failed/retryable rows, including historical
rows that match transient patterns, are initialized exhausted by setting the
table-native attempt counter to `MAX_FAILURE_RETRIES`. They have already
been failing without bounded lifecycle management for weeks; auto-retrying
thousands of ancient rows on upgrade would stampede the AI budget. They then
age into archived via the normal retention step. This converges long-running
installs within one retention window with zero manual surgery and zero retry
storms.

## Compatibility

- Extraction replay ranges have a precise manual recovery path:
  `remem pending list-extraction-ranges --id <positive-id> [--json]`,
  `retry-extraction-ranges --id <positive-id> [--dry-run]`, and
  `quarantine-extraction-ranges --id <positive-id> [--dry-run]`. Explicit
  `--id` conflicts with explicit batch `--project`/`--limit`; implicit batch
  defaults do not make an ID-only command invalid. The list query has no active
  status filter, so `replayed` terminal evidence remains queryable and includes
  the linked replay task id/status/attempt/error without captured payloads or
  provider secrets. Exact dry-run and mutation share the retryable predicate;
  mutation revalidates inside one SQLite transaction and cannot select or
  update a sibling range. Missing, non-positive, archived, active-task, and
  non-retryable IDs fail instead of falling back to the batch path.
  `retry-extraction-ranges --id <positive-id> --acknowledge-quarantine
  [--dry-run]` is the only exception for a quarantined target: the flag
  requires exact ID, reuses the unarchived/no-active-task predicate in dry-run
  and mutation, and never changes the default exact or batch candidate set.
- `remem pending list-failed` / `retry-extraction-ranges` keep working and
  can target archived rows explicitly (`--include-archived`), preserving the
  manual escape hatch for observations and extraction ranges.
- Add an explicit legacy observation replay path:
  `remem pending retry-failed-observations --include-archived --id <id>
  [--host claude-code|codex-cli]` (or `--project <p> --limit <n>
  [--host claude-code|codex-cli]`). The `--id`
  form must run an id-constrained migration/replay path so the selected row is
  the one consumed, not an older pending row from the same project. For
  archived rows it clears `archived_at_epoch`, resets the row to
  `status='pending'`, supplies the explicit `--host` fallback when the row has
  legacy/unknown host identity, and then invokes the legacy pending
  migration/replay flow. Project-wide forms require an explicit or default
  bounded `--limit` and support dry-run preview. This is manual only; it does
  not re-enable automatic pending-observation retries.
- Add extraction-task escape hatches for no-range archived failures:
  `remem pending list-failed-extraction-tasks --include-archived` and
  `remem pending retry-extraction-task --id <id> --include-archived`. The
  retry command reuses the no-range direct requeue path and either clears
  `archived_at_epoch` plus resets the table-native retry counter to 0 before
  work is made pending, or creates a fresh linked retry row for exhausted
  historical rows.
- Add job-specific escape hatches:
  `remem pending list-failed-jobs --include-archived` and
  `remem pending retry-jobs --include-archived --id <id>|--project <p>
  [--limit <n>] [--dry-run]` so an archived job that was misclassified as
  permanent remains explicitly replayable. Project-wide retry has a safe
  default limit, requires dry-run preview for large result sets, and must not
  re-enqueue an unbounded project backlog in one command.
- No change to failure-marking semantics (#365 invariant); W-12 applies to
  the pinned tests around honest marking.

## Phases and Verification

Phase 1: taxonomy columns + `failed_at_epoch` + classification mapping +
back-classification migration across all four surfaces (`cargo test
failure`); migration drift test extends the existing migration test suite.
Phase 2: bounded auto-recovery in the worker + backoff/exhaustion tests with
seeded transient/permanent fixtures, including an extraction task with no
replay range.
Phase 3: archiving step, reporting split in status/doctor/JSON, cleanup
flag, FK-safe replay-range purge, and failure history rollup; doctor fixture
asserting the 1000-archived/2-actionable scenario.

Verify per phase: `cargo fmt --check && cargo check && cargo test`; end-to-
end smoke on a copy of a real long-running store confirming headline counts
drop to actionable-only after one retention window simulation.
