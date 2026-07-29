# Failure Lifecycle Technical Spec

Status: Current contract
Date: 2026-07-28

Tracking:
- Spec/tracking issue: #681
- Related implementation issue: #943

## Existing Implementation Facts

- Extraction replay ranges exist and are user-driven:
  `remem pending list-extraction-ranges` / `retry-extraction-ranges`
  (#426/#660 landed the machinery; doctor reports "27 extraction replay
  ranges retryable" on the reference install).
- Doctor already surfaces raw failed counts (capture-liveness probe and
  pending-queue WARN, #374) but with no age dimension and no 7-day split for
  jobs.
- #365 fixed compression AI failures being mis-marked successful; failure
  marking is honest today. Current extraction, replay, and job failures have
  bounded recovery, and #943 adds a drain-only bridge from eligible legacy
  pending rows into the current capture/extraction pipeline.
- Four failure-bearing surfaces are currently visible to status/doctor:
  `pending_observations` (legacy extraction queue, including
  `status='failed'`), `extraction_tasks` (2919 failed on the reference
  install), `extraction_replay_ranges` (27 retryable ranges on the reference
  install), and the background job queue (`jobs`, 2470 failed). They share the
  lifecycle split below; `pending_observations` remains a retired input surface
  whose residual rows are drained rather than claimed as live queue work.

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

The retired `pending_observations` drain below is a deliberate exception to
attempt exhaustion: it increments the source attempt counter for diagnosis but
caps backoff at 900 seconds and does not silently archive transient residual
evidence. Known-host rows keep retrying at the bounded rate; non-archived
unknown-host rows remain actionable for explicit host repair. Archived rows
excluded by host or class are `admin-required` and use the exact recovery path
below.

Recovery paths are surface-specific. Any retry/requeue path that targets an
archived row must either clear `archived_at_epoch` in the same transaction
before making work pending again, or create a fresh retry row linked to the
archived source; no pending work may retain an archived marker.

- `pending_observations`: use a drain-only bridge; do not restore the deleted
  legacy enqueue, claim, lease-worker, or DTO APIs. An ordinary worker considers
  the bridge only after the current extraction worker reports no ready work.
  `worker --once` may run at most one legacy batch in its process lifetime; a
  daemon may run at most one batch every 60 seconds. Each batch selects at most
  25 oldest rows with a known `claude-code` or `codex-cli` host and one of
  these states: `pending`, expired `processing`, a due non-archived transient
  `failed` row, or a historical archived transient row admitted through the
  same controlled recovery predicate. Permanent and unknown-host rows are not
  automatic candidates; doctor reports archived rows in those classes as
  `admin-required`.

  Before an immediate write transaction starts, automatic and exact replay
  snapshot their source row, while manual batch migration snapshots every
  selected row; all paths resolve optional Git branch metadata before locking.
  The transaction reloads and revalidates each candidate before writing; a
  changed automatic candidate is skipped, while a changed manual candidate
  rolls back the whole batch. Capture receives the explicit precomputed branch
  value, including an explicit no-branch result, so no Git subprocess runs
  while the SQLite write lock is held. Success records the deterministic legacy
  event in `captured_events`, enqueues its `ObservationExtract` task, marks the
  source `migrated`, and clears legacy lease, retry, failure, and archive fields
  atomically. Idempotency converges a repeated attempt on the same
  current-pipeline event/task rather than creating duplicate work. The bridge
  never deletes a source row.

  Any replay failure rolls back the current-pipeline savepoint, increments the
  diagnostic attempt counter, records an exponential `next_retry_epoch` capped
  at 900 seconds, logs class/attempt/outcome/backoff without payload secrets,
  and aborts the remaining batch. The bridge cannot safely infer row-local
  permanence from a shared replay error, so it never changes the source to
  permanent. Archived transient state is cleared only on successful migration,
  never on selection or a failed attempt.

  Archived failed rows excluded from automatic recovery use `remem pending
  recover-archived --id <positive-id> [--host claude-code|codex-cli]
  --dry-run`; apply repeats the exact command without `--dry-run`. It accepts
  no project/batch selector and rejects a missing, non-failed, or non-archived
  target. A known stored host is reused; `host='unknown'` requires the explicit
  host option. Doctor queries this class independently of the global failed-row
  list and prints a bounded oldest-first set with each real ID, stored host,
  failure class, archive time, and concrete preview/apply commands. Apply
  prepares Git metadata before its immediate transaction, revalidates and
  replays only that ID in the transaction, and clears failure/archive state
  only after the captured event, extraction task, and migrated source commit.
  Replay or commit failure leaves the source and current pipeline unchanged.
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
  8-14 day failure continues to affect severity until it archives. Archived
  failed legacy rows that automatic recovery excludes by permanent class or
  unknown host produce a separate doctor `admin-required` finding with
  `list-failed` plus exact `recover-archived` guidance; ordinary archived
  history does not.
- `remem status --json` adds `failures: {actionable_7d, actionable_total,
  transient, permanent, exhausted, archived, historical_archived,
  historical_purged, oldest_actionable_epoch}` per surface.

### 5. Back-classification migration

The schema migration back-fills existing failed rows on all four surfaces:
classify by error string where it matches the mapping; unmatched rows become
`transient`. Pre-existing failed/retryable rows are initialized exhausted by
setting the table-native attempt counter to `MAX_FAILURE_RETRIES`, so the
ordinary extraction/replay/job recovery paths do not stampede on upgrade.

Historical known-host transient `pending_observations`, including rows already
archived by retention, are the deliberate exception. They remain archived
until the drain bridge can admit them while current extraction is idle.
Admission is bounded to 25 rows, once per once-worker process or once per
60-second daemon interval, and transient/shared failures re-enter exponential
backoff. The successful atomic migration clears the legacy attempt,
failure, and archive state; selection or a rolled-back attempt does not.
Permanent and unknown-host historical rows remain unchanged for admin review;
if archived, doctor routes them to exact `recover-archived`.
Non-archived transient rows remain actionable rather than aging into silent
history; known-host rows drain automatically and unknown-host rows require
explicit host repair. This exception drains unique legacy evidence without
reviving the old queue or creating an upgrade-time retry storm.

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
  update a sibling range. Missing, non-positive, active-task, and non-retryable
  IDs fail instead of falling back to the batch path; archived IDs also fail
  unless the exact archived-recovery opt-in below is present.
  `retry-extraction-ranges --id <positive-id> --acknowledge-quarantine
  [--dry-run]` is the only exception for a quarantined target: the flag
  requires exact ID, reuses the unarchived/no-active-task predicate in dry-run
  and mutation, and never changes the default exact or batch candidate set.
  If that same target has since archived, `--include-archived` is also required;
  it is exact-ID-only and reuses the same no-active-task predicate. The pending
  command permits this archived combination only with `--dry-run`; a mutating
  invocation fails with guidance to use the locked exact worker. No batch path
  receives either opt-in, and no unlocked command clears an archived range.
- `remem worker --once --replay-range-id <positive-id>
  --acknowledge-quarantine --include-archived --profile <name>` validates the
  profile and acquires the worker singleton before changing the range. While
  holding the lock, one SQLite transaction revalidates the exact range,
  requeues it, and claims the returned replay task with the ordinary
  pending/retry-due predicate; the pending row is never committed without the
  exact lease. A held daemon lock, future retry time, or identity race fails
  before fallback. The exact processor uses the validated in-memory profile
  for its single attempt. Full-range done follows the normal success
  transition; partial coverage, defer, wait, timeout, provider error, or
  another non-success atomically leaves the
  replay task failed/archived and the range quarantined/archived. Expired lease
  recovery recognizes exact-replay owners and applies the same archived
  quarantine outcome, so interruption never creates daemon-claimable work with
  a default profile. This mode does not run lifecycle maintenance, priority
  fallback, jobs, embedding backfill, or a second extraction task; ordinary
  worker modes keep their existing drain behavior.
- `remem pending list-failed` / `retry-extraction-ranges` keep working; the
  latter can validate an archived extraction range only by exact ID with
  `--include-archived --dry-run`; the locked exact worker is the sole mutating
  archived-range escape hatch.
- Add one exact archived legacy observation replay path:
  `remem pending recover-archived --id <positive-id>
  [--host claude-code|codex-cli] --dry-run`; apply removes `--dry-run`.
  The command accepts only one archived failed `pending_observations` row and
  never falls back to an older row, project, or batch selection. A row stored
  with `host='unknown'` requires the explicit host on preview and apply. The
  mutating form revalidates the same ID and atomically records its current
  captured event/extraction task, marks the source migrated, and only then
  clears failure/archive state. Any error rolls back all current-pipeline
  writes and preserves every source field. Doctor labels archived permanent or
  unknown-host rows `admin-required` and directly provides bounded candidate
  details plus these exact commands. This supplements the automatic drain
  without restoring a legacy enqueue or claim API.
- Current limitation: no public pending subcommand recovers an archived
  no-range extraction task. Such rows remain inspectable in status/doctor and
  retained history, but the CLI must not advertise a recovery command until an
  exact-ID implementation and its active-task revalidation land.
- Current limitation: no public pending subcommand recovers archived jobs.
  They remain inspectable in status/doctor and retained history. Any future
  recovery command must use exact or bounded selection, dry-run for a project
  batch, and the ordinary active-identity/coalescing rules before it is
  documented as executable.
- No change to failure-marking semantics (#365 invariant); W-12 applies to
  the pinned tests around honest marking.

## Phases and Verification

Phase 1: taxonomy columns + `failed_at_epoch` + classification mapping +
back-classification migration across all four surfaces (`cargo test
failure`); migration drift test extends the existing migration test suite.
Phase 2: bounded auto-recovery in the worker + backoff/exhaustion tests with
seeded transient/permanent fixtures, including an extraction task with no
replay range. Legacy drain fixtures cover current-work priority, oldest-first
selection, the 25-row cap, one batch per once-worker process, one batch per
60-second daemon interval, and known-host pending/expired-processing/due
transient rows.
Phase 3: archiving step, reporting split in status/doctor/JSON, cleanup
flag, FK-safe replay-range purge, and failure history rollup; doctor fixture
asserting the 1000-archived/2-actionable scenario. Historical archived
transient pending fixtures prove controlled admission without clearing archive
state before an atomic success. Permanent and unknown-host fixtures remain
admin-visible; archived controls produce `admin-required` exact-recovery
guidance. Exact recovery tests cover dry-run, wrong state, missing ID, required
unknown-host override, atomic success, and failure rollback. Fault injection
proves capped backoff, preserved first-failure and archive timestamps,
partial-batch commit, remaining-batch abort, and no captured event or extraction
task leakage for the failing row.

Verify per phase: `cargo fmt --check && cargo check && cargo test`; end-to-
drop to actionable-only after one retention window simulation. A process-level
fault-injection test starts a worker, kills it after a failed legacy backlog is
durable, restarts the worker, proves singleton reacquisition, and proves
backlog zero. Adjacent tests prove current extraction drains first, daemon and
once-worker rate limits, idempotent migration, and zero automatic deletion.
