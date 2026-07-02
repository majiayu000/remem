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
- Two failure populations exist: `extraction_tasks` (2919 failed on the
  reference install) and the background job queue (`db_job`, 2470 failed).
  Both accumulate without retention.

## Design

### 1. Failure taxonomy

New columns on both failed populations (migration):
`failure_class TEXT` (`transient` | `permanent`), `retry_attempts INTEGER
NOT NULL DEFAULT 0`, `archived_at_epoch INTEGER NULL`.

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
(`next_retry = failed_at + base * 2^attempts`, attempts capped at
`MAX_FAILURE_RETRIES`, default 3). Extraction-task retries route through the
existing replay-range machinery rather than a parallel path; job retries
re-enqueue the job. Every attempt logs class, attempt count, and outcome.
On cap exhaustion the row is marked exhausted (attempts = cap) and becomes
eligible for archiving. Permanent-class rows are archive-eligible
immediately.

### 3. Retention / archiving

A worker maintenance step transitions eligible rows to archived
(`archived_at_epoch` set) once they are older than
`failure_retention_days` (config, default 14). Archiving is a state
transition; rows and their error strings remain queryable. Aggregate
history (per class/day counts) is preserved via the existing stats query
layer — no separate history table in v1.

`remem cleanup --archived-failures[=<days>]` (default horizon 90 days)
deletes archived rows older than the horizon, printing counts (explicit
purge only).

### 4. Reporting split

- Status/doctor headline counters exclude archived rows. Actionable =
  not-archived failures; the probe prints `actionable (7d)`, oldest
  actionable age, per-class counts, and `archived: <n>` as a secondary line.
- Severity: FAIL/WARN thresholds evaluate actionable counts only; a store
  with thousands of archived and zero actionable failures reports ok.
- `remem status --json` adds `failures: {actionable_7d, actionable_total,
  transient, permanent, exhausted, archived, oldest_actionable_epoch}` per
  population.

### 5. Back-classification migration

The schema migration back-fills existing failed rows: classify by error
string where it matches the mapping; unmatched rows become
`transient` with `retry_attempts = MAX_FAILURE_RETRIES` (exhausted — they
have been failing without retries for weeks; auto-retrying thousands of
ancient rows on upgrade would stampede the AI budget). They then age into
archived via the normal retention step. This converges long-running installs
within one retention window with zero manual surgery and zero retry storms.

## Compatibility

- `remem pending list-failed` / `retry-extraction-ranges` keep working and
  can target archived rows explicitly (`--include-archived`), preserving the
  manual escape hatch.
- No change to failure-marking semantics (#365 invariant); W-12 applies to
  the pinned tests around honest marking.

## Phases and Verification

Phase 1: taxonomy columns + classification mapping + back-classification
migration (`cargo test failure`); migration drift test extends the existing
migration test suite.
Phase 2: bounded auto-recovery in the worker + backoff/exhaustion tests with
seeded transient/permanent fixtures.
Phase 3: archiving step, reporting split in status/doctor/JSON, cleanup
flag; doctor fixture asserting the 1000-archived/2-actionable scenario.

Verify per phase: `cargo fmt --check && cargo check && cargo test`; end-to-
end smoke on a copy of a real long-running store confirming headline counts
drop to actionable-only after one retention window simulation.
