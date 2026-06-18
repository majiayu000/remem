# Observation Drain, Host Isolation, and Worker Scheduler Spec

Date: 2026-05-05

## Summary

`remem` currently captures tool events faster than it consumes pending observations. The
immediate production symptom is a large `pending_observations` backlog even though the
`jobs` queue is empty and workers complete successfully.

The long-term design is a layered fix:

1. Make each observation job drain multiple pending batches within a strict budget.
2. Add `host` to pending and job identity so Claude Code and Codex events never mix.
3. Add a worker scheduler/daemon mode that continuously drains stale work, while keeping
   Stop hooks as a fallback wake-up path.

This keeps hooks fast, preserves session context, avoids oversized AI calls, and makes
queue recovery independent of whether a future Stop hook fires.

## Current State

### Observed runtime state

- `cargo check`, `cargo test`, `cargo build --release`, and CLI startup pass.
- Codex hooks and MCP are registered.
- Claude Code hooks/MCP may be absent on a given machine; that does not affect Codex.
- `pending_observations` can grow into hundreds of rows while `jobs` has no failed or
  pending work.
- The largest backlog observed was a single Codex session under
  `/Users/lifcc/Desktop/code/AI/tool/harness`, all `Bash` events.

### Relevant code path

- `observe` inserts events and pending observations:
  - `src/observe/hook.rs`
- Stop hook enqueues one observation job, one summary job, and one compress job:
  - `src/summarize/summary_job/hook.rs`
- `worker` handles an observation job by calling `observe_flush::flush_pending` once:
  - `src/worker.rs`
- `flush_pending` claims at most `FLUSH_BATCH_SIZE` rows:
  - `src/observe_flush/batch.rs`
  - `src/observe_flush/constants.rs`
- `claim_pending` filters by `session_id` only:
  - `src/db_pending/claim.rs`
- Job dedupe prevents a new pending/processing job with the same
  `job_type + project + session_id`:
  - `src/db_job/enqueue.rs`

## Problem Statement

The current observation job means "flush one batch". In practice it needs to mean
"advance this session's pending observation queue until it is empty or this worker has
spent its allowed budget".

The current model has four structural issues:

1. One observation job processes at most 15 rows.
2. Follow-up work is not automatically scheduled when rows remain.
3. Pending rows can remain `processing` after a worker timeout because only job leases
   are globally recovered.
4. Queue identity does not include host, even though Claude Code and Codex have different
   capture semantics.

## Goals

- Keep `observe` hooks fast and AI-free.
- Preserve per-session memory quality.
- Prevent Claude Code and Codex events from being consumed in the same batch.
- Bound AI work per worker invocation.
- Make backlog drain eventually complete without manual database intervention.
- Improve `status` and `doctor` so queue health reflects real state.
- Keep the existing Rust single-binary architecture.
- Introduce daemon/scheduler support only after the bounded drain behavior is stable.

## Non-Goals

- Do not call AI from `observe`.
- Do not increase `FLUSH_BATCH_SIZE` enough to hide the bug.
- Do not drain by project alone.
- Do not introduce a separate service or microservice.
- Do not remove Stop hook scheduling.
- Do not require Claude Code to be installed for Codex to work.

## Design Principles

1. Capture is fast, asynchronous, and loss-resistant.
2. Consumption is budgeted, retryable, and observable.
3. Exact consumption identity is `host + project + session_id`.
4. Time windows are scheduler boundaries, not semantic ownership.
5. Summary generation must not be starved by observation backlog.
6. Daemon mode improves reliability but Stop hooks remain the compatibility fallback.

## Phase 1: Bounded Observation Drain

### New semantics

An observation job drains ready pending rows for one session under a budget:

```text
process observation job:
    recover expired pending leases

    while drain budget remains:
        claim up to FLUSH_BATCH_SIZE ready rows for this session
        if no rows:
            return Drained

        flush claimed rows
        delete successful rows
        retry transient failures
        fail permanent non-observation rows

    if ready rows remain:
        return NeedsFollowUp

    return Drained
```

### Budget constants

Add constants in `src/observe_flush/constants.rs`:

```rust
pub(crate) const FLUSH_DRAIN_MAX_BATCHES: usize = 4;
pub(crate) const FLUSH_DRAIN_MAX_SECS: u64 = 240;
pub(crate) const OBSERVATION_FOLLOW_UP_PRIORITY: i64 = 150;
```

Rationale:

- Keep each AI request small with `FLUSH_BATCH_SIZE = 15`.
- Let one job make real progress.
- Keep summary priority `100` able to run before follow-up observation priority `150`.
- Leave room under the current worker timeout.

`FLUSH_DRAIN_MAX_SECS` must remain lower than `JOB_TIMEOUT_SECS` in `src/worker.rs`.

### Follow-up scheduling

The follow-up job must be enqueued only after the current job is marked `done`.

Reason: `enqueue_job` dedupes `pending` and `processing` jobs. If the current job is still
`processing`, a follow-up enqueue will return the current job id instead of creating new
work.

Worker flow:

```text
let outcome = process_job(job)

if outcome succeeds:
    mark_job_done(job)
    if outcome.needs_follow_up:
        enqueue observation job with OBSERVATION_FOLLOW_UP_PRIORITY
```

### Drain outcome

Add an internal outcome type:

```rust
pub(crate) enum ObservationDrainOutcome {
    Drained,
    NeedsFollowUp,
}
```

`worker::process_job` should return a richer outcome than `Result<()>` so the caller can
schedule follow-up after `mark_job_done`.

### Remaining-row query

Add query helpers in `src/db_pending/query.rs`:

- `count_ready_pending_for_session(conn, session_id) -> Result<i64>`
- later phase: `count_ready_pending_for_identity(conn, host, project, session_id) -> Result<i64>`
- `oldest_ready_pending_epoch(...) -> Result<Option<i64>>`

The Phase 1 query may keep session-only behavior to minimize schema churn, but Phase 2
must replace it with the exact identity.

### Expired pending lease recovery

Add `release_expired_pending_claims(conn) -> Result<usize>` in
`src/db_pending/claim.rs`.

It should reset expired `processing` pending rows:

```sql
UPDATE pending_observations
SET status = 'pending',
    lease_owner = NULL,
    lease_expires_epoch = NULL,
    updated_at_epoch = ?1
WHERE status = 'processing'
  AND lease_expires_epoch IS NOT NULL
  AND lease_expires_epoch < ?1
```

Call it before claiming pending rows and from worker startup next to
`requeue_stuck_jobs`.

### Failure behavior

Transient AI errors:

- Reset pending rows to `status='pending'`.
- Set `next_retry_epoch`.
- Store `last_error`.
- Let follow-up or scheduler pick them up later.

Permanent parse/no-observation behavior:

- Keep current behavior for empty action-batch output: mark rows `failed`.
- Keep current Task skip/fail semantics.
- Do not silently delete rows when no observation is produced unless the existing code has
  an explicit skip rule for that class.

### Tests

Add or update tests:

- `worker_drains_multiple_observation_batches`
- `worker_reenqueues_follow_up_after_mark_done`
- `follow_up_priority_allows_summary_to_run_first`
- `expired_pending_processing_rows_are_released`
- `observation_job_does_not_enqueue_follow_up_when_empty`
- `observation_job_retries_transient_ai_failure`

## Phase 2: Host Isolation and Exact Consumption Identity

### Schema additions

Add migration `v003_host_identity.sql`:

```sql
ALTER TABLE pending_observations
ADD COLUMN host TEXT NOT NULL DEFAULT 'unknown';

ALTER TABLE jobs
ADD COLUMN host TEXT NOT NULL DEFAULT 'unknown';

CREATE INDEX IF NOT EXISTS idx_pending_identity_claim
ON pending_observations(host, project, session_id, status, next_retry_epoch, lease_expires_epoch, id);

CREATE INDEX IF NOT EXISTS idx_jobs_identity_state
ON jobs(host, project, session_id, job_type, state, created_at_epoch DESC);
```

Update `src/migrate/types.rs` to include version 3.

### Host source

Use adapter identity as host:

- Claude Code: `claude-code`
- Codex CLI: `codex-cli`

`observe` already knows the adapter returned by `detect_adapter`. Pass
`adapter.name()` into `enqueue_pending`.

Stop hook payloads do not currently persist host. Add host to observation job payload and
job row:

```json
{
  "host": "codex-cli",
  "session_id": "...",
  "project": "..."
}
```

For Stop hook host detection:

- Prefer `REMEM_CONTEXT_HOST` or a new explicit `REMEM_HOOK_HOST`.
- Fall back to executor hint:
  - `REMEM_SUMMARY_EXECUTOR=codex-cli` means `codex-cli`.
  - `REMEM_SUMMARY_EXECUTOR=claude-cli` means `claude-code`.
- If neither is present, use `unknown` and keep compatibility.

### Enqueue APIs

Change APIs:

```rust
enqueue_pending(conn, host, session_id, project, ...)
enqueue_job(conn, host, job_type, project, session_id, payload_json, priority)
```

Update dedupe:

```sql
WHERE host = ?1
  AND job_type = ?2
  AND project = ?3
  AND COALESCE(session_id, '') = COALESCE(?4, '')
  AND state IN ('pending', 'processing')
```

### Claim APIs

Replace session-only claim with exact identity:

```rust
claim_pending(conn, host, project, session_id, limit, lease_owner, lease_secs)
```

SQL:

```sql
WHERE host = ?host
  AND project = ?project
  AND session_id = ?session_id
  AND status = 'pending'
  AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?now)
  AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?now)
```

### Compatibility for existing rows

Existing rows get `host='unknown'`.

Compatibility options:

1. Drain existing `unknown` rows before enabling host-strict claim.
2. During migration window, allow exact host claims to also claim `unknown` rows only when
   `project + session_id` matches.

Recommended:

- Implement compatibility option 2.
- Add a `doctor` warning when `unknown` host rows remain.
- Remove compatibility only in a future major migration.

### Tests

Add tests:

- `claim_pending_respects_host_project_session_identity`
- `same_session_different_project_not_claimed`
- `same_session_different_host_not_claimed`
- `legacy_unknown_host_rows_can_be_claimed_by_matching_identity`
- `job_dedupe_includes_host`
- `stale_pending_sessions_group_by_host_project_session`

## Phase 3: Scheduler and Daemon

### Desired mode

Use the existing single binary:

```bash
remem worker
```

Long-running worker mode should:

- Process ready jobs.
- Requeue expired job leases.
- Release expired pending leases.
- Scan stale pending rows and enqueue observation jobs.
- Emit heartbeat.
- Sleep when idle.

Stop hook remains:

- Enqueue observation/summary/compress jobs.
- Spawn `worker --once` only when no healthy daemon heartbeat exists.

### Scheduler tick

Every idle tick:

1. Recover expired job leases.
2. Recover expired pending leases.
3. Find stale pending identities older than a threshold.
4. Enqueue observation jobs for those identities.

Add helper:

```rust
get_stale_pending_identities(conn, age_secs, limit)
    -> Vec<PendingIdentity { host, project, session_id, ready_count, oldest_epoch }>
```

Use `host + project + session_id`, not project-only.

### Daemon heartbeat

Add a lightweight heartbeat table:

```sql
CREATE TABLE IF NOT EXISTS worker_heartbeats (
    owner TEXT PRIMARY KEY,
    pid INTEGER,
    started_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);
```

Alternative: write a heartbeat file under `~/.remem/worker-heartbeat.json`.

Preferred for this project: database table, because `doctor` and `status` already open the
database.

### Stop fallback

Stop hook should check daemon health:

- If heartbeat age is healthy, enqueue jobs and return.
- If no healthy heartbeat, spawn `worker --once` as today.

This preserves current behavior when daemon is not installed.

### Installation path

Do not enable daemon by default until manual mode is proven.

Rollout:

1. Manual:
   - `remem worker`
   - `remem doctor`
2. Opt-in install:
   - `remem install --worker-daemon`
3. Stable default:
   - enable daemon for selected target if user opts in or if install policy changes later.

macOS daemon should use `launchd` under `~/Library/LaunchAgents`.
Linux should use user `systemd` when available.

### Executor hints

Daemon processes do not naturally inherit hook env variables. Therefore, jobs must carry
host/executor hints.

Add to job payload or columns:

- `host`
- `summary_executor`
- `flush_executor`

Minimum:

- `host` column in Phase 2.
- Resolve executor from host at execution time.

Future:

- explicit `executor` column if users configure non-default execution.

### Tests

Add tests:

- `scheduler_enqueues_stale_pending_identity`
- `scheduler_does_not_enqueue_duplicate_processing_job`
- `healthy_daemon_skips_stop_spawn`
- `missing_daemon_uses_stop_fallback_spawn`
- `worker_heartbeat_updates_in_loop`

## Status and Doctor Improvements

Current stats should be expanded and corrected.

### Bug fix

`query_system_stats` checks stuck jobs with `state = 'running'`, but the real state is
`processing`.

Fix:

```sql
SELECT COUNT(*) FROM jobs
WHERE state = 'processing'
  AND lease_expires_epoch < strftime('%s', 'now')
```

### New metrics

Expose:

- pending ready count
- pending delayed count
- pending processing count
- pending failed count
- expired pending processing count
- oldest ready pending age
- top backlog identities
- jobs pending count
- jobs processing count
- jobs failed count
- expired processing jobs
- daemon heartbeat age

### CLI output

`remem status` should show:

```text
Pending observations:
  Ready:        734
  Delayed:        0
  Processing:     0
  Failed:         0
  Oldest ready:   25h

Top backlog identities:
  734  codex-cli  /path/project  session-id

Jobs:
  Pending:        0
  Processing:     0
  Failed:         0
  Stuck:          0

Worker:
  Daemon:         healthy, last heartbeat 3s ago
```

`remem doctor` should warn when:

- ready pending count exceeds threshold
- oldest ready pending exceeds threshold
- processing pending lease expired
- daemon expected but not healthy
- unknown host rows remain after migration

## Claude Code Impact

Claude Code captures a broader event set:

- `Write`
- `Edit`
- `NotebookEdit`
- `Bash`
- `Task`

It also has `UserPromptSubmit` in install configuration, unlike Codex.

Expected impact:

- Positive: Claude pending rows drain more reliably.
- Positive: stale recovery handles missed Stop hooks.
- Positive: host isolation prevents Codex Bash streams from mixing with Claude file/task
  events.
- Neutral: native memory sync remains in `observe`; this spec does not move it.
- Risk: daemon environments may not have Claude CLI credentials or PATH.

Mitigation:

- Keep Stop fallback.
- Store host/executor hints.
- Prefer absolute binary paths in install output.
- Let `doctor` report daemon executor availability.

## Codex Impact

Codex capture is currently Bash-focused:

- `PostToolUse(Bash)`
- short observe timeout
- no native file-edit capture through this hook path
- Stop hook sets Codex executor env for summary and flush

Expected impact:

- Strongly positive: high-volume Bash sessions no longer accumulate unbounded pending rows.
- Stop hook remains fast because AI stays in worker.
- Follow-up priority prevents observation backlog from starving summary jobs.
- Daemon scheduler prevents long sessions from depending on a future Stop hook.

Risk:

- Daemon launched outside Codex may not know to use Codex executor.

Mitigation:

- Persist host in jobs.
- Resolve flush executor from host.
- Keep Stop fallback until daemon executor behavior is verified.

## Data Integrity and Idempotency

- Pending rows are deleted only after observations are persisted.
- Delete remains lease-owner scoped.
- Follow-up jobs are enqueued after current job completion to avoid dedupe self-collision.
- Transient failures retry pending rows with backoff.
- Permanent failures remain inspectable through existing pending admin commands.
- Host migration must not orphan old `unknown` rows.

## Security Considerations

- Do not execute shell commands from database fields.
- Use parameterized SQL for all claim, retry, and scheduler queries.
- Do not store secrets in job payloads.
- Do not broaden daemon environment with user shell startup scripts.
- Use absolute paths for installed binaries.
- `doctor` may report missing executor credentials but must not print secret values.

## Implementation Plan

### Step 1: Queue correctness patch

Files:

- `src/observe_flush/constants.rs`
- `src/observe_flush/batch.rs`
- `src/db_pending/claim.rs`
- `src/db_pending/query.rs`
- `src/worker.rs`
- `src/db_query/stats.rs`
- `src/doctor/database.rs`
- `src/cli/actions/query/status.rs`

Deliverables:

- bounded drain
- follow-up scheduling after `mark_job_done`
- expired pending lease recovery
- stuck jobs stats bug fix
- basic queue metrics

Validation:

```bash
cargo test worker_drains_multiple_observation_batches -- --nocapture
cargo test expired_pending_processing_rows_are_released -- --nocapture
cargo test check_pending_queue_reports_shared_counts -- --nocapture
cargo check
cargo test
```

### Step 2: Host identity migration

Files:

- `src/migrations/v003_host_identity.sql`
- `src/migrate/types.rs`
- `src/db_pending/types.rs`
- `src/db_pending/queue.rs`
- `src/db_pending/claim.rs`
- `src/db_pending/query.rs`
- `src/db_job/enqueue.rs`
- `src/db_models.rs`
- `src/observe/hook.rs`
- `src/summarize/summary_job/hook.rs`
- `src/worker.rs`

Deliverables:

- host column on pending and jobs
- host-aware enqueue/dedupe/claim
- legacy unknown-host compatibility
- host-aware job payload

Validation:

```bash
cargo test claim_pending_respects_host_project_session_identity -- --nocapture
cargo test same_session_different_host_not_claimed -- --nocapture
cargo test job_dedupe_includes_host -- --nocapture
cargo test full_migration_on_empty_db -- --nocapture
cargo test real_queries_work_on_upgraded_db -- --nocapture
cargo check
cargo test
```

### Step 3: Scheduler mode

Files:

- `src/worker.rs`
- `src/db_pending/query.rs`
- `src/db_job/enqueue.rs`
- `src/doctor/database.rs`
- `src/cli/actions/query/status.rs`
- optional new module: `src/worker/scheduler.rs`

Deliverables:

- stale pending identity scanner
- daemon heartbeat
- Stop fallback only when daemon unhealthy
- status/doctor daemon reporting

Validation:

```bash
cargo test scheduler_enqueues_stale_pending_identity -- --nocapture
cargo test healthy_daemon_skips_stop_spawn -- --nocapture
cargo check
cargo test
```

### Step 4: Optional managed daemon install

Files:

- `src/install`
- `README.md`
- `README.zh-CN.md`

Deliverables:

- opt-in daemon install
- uninstall cleanup
- doctor instructions

Validation:

```bash
cargo test install -- --nocapture
cargo check
cargo test
```

## Rollout Plan

1. Land Step 1 and verify it drains the current backlog with manual `remem worker`.
2. Run `remem doctor` and confirm pending ready count decreases.
3. Land Step 2 and verify legacy `unknown` rows are still claimable.
4. Run mixed Codex and Claude fixture tests.
5. Land Step 3 behind existing worker command behavior.
6. Manually run daemon mode before adding managed install.
7. Add opt-in install only after manual daemon behavior is stable.

## Acceptance Criteria

Functional:

- A single observation job can process more than one batch.
- Large pending backlog eventually drains without manual SQL.
- Summary jobs are not starved by observation follow-ups.
- Expired pending processing rows recover automatically.
- Host-specific claim prevents Claude/Codex mixing.
- Existing `unknown` rows remain drainable.

Observability:

- `status` reports ready/delayed/processing/failed pending rows.
- `doctor` reports accurate stuck jobs using `processing`.
- Top backlog identities are visible.
- Daemon health is visible once daemon mode is added.

Compatibility:

- Codex hooks remain fast.
- Claude Code hooks remain compatible.
- Stop hook fallback remains available.
- Existing databases migrate without data loss.

Validation:

- `cargo check` passes.
- `cargo test` passes.
- Targeted queue, migration, worker, and install tests pass.

## Open Questions

1. Should `FLUSH_DRAIN_MAX_BATCHES` default to 4 or 8?
2. Should `FLUSH_DRAIN_MAX_SECS` be 180, 240, or tied to `JOB_TIMEOUT_SECS`?
3. Should daemon heartbeat live in SQLite or a JSON file?
4. Should executor hints be columns or only job payload fields?
5. Should old `unknown` host rows be migrated heuristically from hook logs, or only handled
   through compatibility claim?

Recommended defaults:

- `FLUSH_DRAIN_MAX_BATCHES = 4`
- `FLUSH_DRAIN_MAX_SECS = 240`
- heartbeat in SQLite
- `host` as a column; executor hint can start in payload
- keep `unknown` compatibility instead of heuristic rewrite

