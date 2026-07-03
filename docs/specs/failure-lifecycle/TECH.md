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

Recovery paths are surface-specific:

- `pending_observations`: re-enter the existing pending-observation claim
  path by setting `status='pending'`, clearing lease fields, and setting
  `next_retry_epoch`; this keeps legacy failed rows from remaining a
  permanent headline WARN.
- `extraction_replay_ranges`: invoke the existing
  `retry_extraction_replay_ranges` machinery for retryable ranges.
- `extraction_tasks` with a replay range: route through that range.
- `extraction_tasks` without a replay range: requeue the original task
  directly by setting `status='pending'`, clearing lease fields, and setting
  `next_retry_epoch`; no-range transient failures therefore have an explicit
  recovery path instead of staying actionable forever.
- `jobs`: re-enqueue the failed job by setting `state='pending'`, clearing
  lease fields, and setting `next_retry_epoch`.

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
- for a replay range and its archived replay task purged in the same
  transaction, nullable `extraction_tasks.replay_range_id` references are
  cleared before deleting the range, then archived tasks are deleted.

### 4. Reporting split

- Status/doctor headline counters exclude archived rows across all four
  surfaces. Actionable = not-archived failures plus retryable replay ranges;
  the probe prints `actionable (7d)`, oldest actionable age, per-class
  counts, and `archived: <n>` as a secondary line.
- Severity: FAIL/WARN thresholds evaluate actionable counts only; a store
  with thousands of archived and zero actionable failures reports ok.
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

- `remem pending list-failed` / `retry-extraction-ranges` keep working and
  can target archived rows explicitly (`--include-archived`), preserving the
  manual escape hatch for observations and extraction ranges.
- Add job-specific escape hatches:
  `remem pending list-failed-jobs --include-archived` and
  `remem pending retry-jobs --include-archived --id <id>|--project <p>` so an
  archived job that was misclassified as permanent remains explicitly
  replayable.
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
