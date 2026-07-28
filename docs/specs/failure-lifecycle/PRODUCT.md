# Failure Lifecycle Product Spec

Status: Current contract
Date: 2026-07-28

Tracking:
- Spec/tracking issue: #681
- Related: #426/#660 (extraction replay machinery), #374 (doctor probes),
  #365 (honest failure marking), #943 (legacy pending drain bridge)

## Problem

Terminal failure counters grow monotonically with no retention or
auto-recovery policy. A live long-running install (2026-07-02) shows 2919
failed extraction tasks and 2470 failed jobs, with 27 replay ranges
retryable; current doctor/status paths also surface failed legacy
`pending_observations`. Nothing distinguishes "failed last night,
actionable" from "failed six weeks ago on a since-fixed version,
historical".

Once failed counts reach the thousands, every doctor run cries wolf. The
alarm-fatigue effect is documented in practice: the same doctor WARN lines
have had to be re-interpreted repeatedly across working sessions instead of
being self-explanatory. The one failure that matters — a #674-class silent
pipeline stall — drowns in the noise. This directly degrades the trust
surface that #381/#383 evidence collection depends on.

## Goals

- Failures carry a class (transient vs permanent) so the system and the user
  know what is worth retrying.
- Transient failures auto-recover through the existing replay machinery with
  bounded, logged retries. The retired pending-observation queue is drained
  into that current pipeline only while current extraction work is idle; it
  is not revived as a second queue.
- Failures that exhaust retries or are permanent leave the headline counters
  after a retention window, without losing audit history.
- Doctor and status answer "what needs my attention now" with an actionable
  count and the age of the oldest actionable failure, separately from
  archived history.

## Non-Goals

- No relaxation of failure marking: failures stay failures (#365 stays
  fixed); archiving is a lifecycle transition, not a reclassification to
  success.
- No automatic retry of permanent-class failures.
- No automatic row deletion; purging archived rows is an explicit
  `remem cleanup` action, consistent with invalidate-never-delete.

## User-Visible Behavior

- `remem status` / `remem doctor` split failure reporting into
  `actionable total`, `actionable 7d`, and `archived history`; FAIL/WARN
  severity keys off actionable total (all non-archived failures), and shows
  the oldest actionable failure age.
- `remem status --json` exposes per-class counts (transient/permanent),
  attempt counts, and archived totals.
- Ordinary workers drain eligible legacy pending rows only when no current
  extraction task is ready. A once worker may drain at most one batch per
  process; a daemon may drain at most one batch of 25 every 60 seconds.
- Eligible legacy rows have a known Claude Code or Codex host and are pending,
  expired-processing, or due transient failures. The same controlled path may
  recover historical archived transient rows. Permanent and unknown-host rows
  are never automatic candidates.
- Doctor reports archived failed legacy rows excluded from the automatic bridge
  as `admin-required`. Operators inspect the row, preview exact recovery with
  `remem pending recover-archived --id <id> --dry-run`, and apply by removing
  `--dry-run`; an unknown stored host requires an explicit Claude Code or Codex
  host on both commands.
- A successful drain atomically records the idempotent captured event,
  enqueues the current extraction task, marks the legacy source migrated, and
  clears its old failure/archive state. It never automatically deletes a row.
- Successful drain batches log their surface, outcome, and migrated count.
  A failed replay logs surface, transient class, attempt, outcome, and capped
  backoff, then stops the rest of that batch. The bridge does not infer a
  row-local permanent classification from a shared replay error. Exhaustion
  and archiving transitions on the other recovery surfaces are logged at
  error/info respectively (U-29: no silent state changes).
- A rejected lease-owned job transition never becomes an in-memory success.
  A missing target fails loudly and remains absent, so it contributes no
  processing or stuck row to status/doctor. For an existing wrong-owner,
  reclaimed, expired, or otherwise ineligible target, every persisted field
  remains unchanged; if that row is still `processing`, status/doctor continue
  to show it as processing and then stuck after its unchanged lease expires.
- A non-Summary active duplicate reconciled by the v069 job-queue migration
  becomes a permanent, non-archived actionable failure. Its real attempt/error
  evidence remains queryable and it follows the existing retention/archive
  lifecycle instead of being hidden, deleted, or reported as exhausted.
- If a due failed job collides with equivalent active work, that canonical
  work carries execution forward. The source remains a failed, permanent,
  queryable audit row with its real attempt count unchanged and does not enter
  automatic retry again; logs identify the safe source/canonical ids without
  exposing the original error text.
- Retired legacy Summary jobs never enter generic job auto-recovery. Candidate
  selection excludes them, and the per-row recovery guard returns an explicit
  retired/skipped outcome for any defensive direct input while leaving every
  persisted audit field and recovery counter unchanged.
- `remem cleanup --archived-failures[=<days>]` purges archived rows older
  than the horizon, reporting what was removed.
- Operators can inspect, preview, retry, or quarantine one extraction replay
  range by positive ID. Exact listing remains available after the range reaches
  terminal `replayed` state and returns the linked replay task's status,
  attempts, and bounded error evidence. Exact mutation never selects or changes
  a sibling range and never falls back to a batch operation. A quarantined
  range remains ineligible by default and can be retried only by combining its
  exact ID with an explicit quarantine acknowledgement; batch retry never
  includes quarantined ranges.

## Acceptance Criteria

- Seeded transient extraction/job failure auto-recovers through replay/requeue
  with backoff; attempts and class are visible in logs and
  `remem status --json`.
- With no ready current extraction work, an ordinary worker selects at most 25
  oldest eligible known-host legacy rows: pending, expired-processing, due
  transient failed, or controlled historical archived transient rows. It
  migrates each successful row atomically into `captured_events` plus an
  `ObservationExtract` task and clears legacy failure/archive state.
- A once worker runs at most one legacy drain batch per process. A daemon runs
  at most one batch per 60 seconds. Ready current extraction work always wins,
  so a historical backlog cannot displace current capture processing.
- A process-level fault-injection test starts a worker, kills it while failed
  legacy backlog is durable, restarts it, proves the singleton can be
  reacquired, and asserts that backlog reaches zero.
- A shared or transient drain error rolls back the current row, records
  exponential backoff capped at 900 seconds, and stops the batch. Other replay
  errors take the same conservative path instead of being guessed permanent.
  Rows already classified permanent and unknown-host rows remain unchanged and
  admin-visible.
- The bridge adds no legacy enqueue or claim API and never automatically
  deletes legacy rows. `recover-archived` accepts only one exact failed,
  archived legacy row; dry-run performs no writes, unknown host requires
  `--host claude-code|codex-cli`, success atomically replays and clears
  failure/archive state, and failure preserves the source.
- Seeded permanent failure never auto-retries and archives after the window;
  headline counters drop while the row remains queryable until explicit
  cleanup and aggregate history remains queryable after cleanup.
- A missing-row lease transition fails with an explicit missing diagnostic,
  emits no success event, leaves the row absent, and adds no processing/stuck
  count. Rejection of an existing row leaves every persisted field unchanged;
  when that row is still processing it remains visible and is reported as
  stuck after the unchanged lease expires.
- A seeded non-Summary v069 duplicate is permanent, non-archived and
  actionable, preserves its original attempt/error evidence without false
  exhaustion, and later archives through the existing retention lifecycle.
- A seeded job retry that collides with equivalent active work converges on the
  canonical active job, preserves the source error and real attempt count in a
  permanent failed row, emits only safe collision metadata, and does not retry
  that source again.
- A batch containing a due-like retired Summary and an unrelated retryable job
  excludes the Summary while recovering the unrelated job. Direct per-row
  Summary input is skipped explicitly with byte/value-identical persisted
  fields and no requeued/coalesced count.
- Exact listing of a terminal replay range returns that range and its linked
  replay-task evidence. Exact retry/quarantine revalidates the same ID in one
  transaction, changes only that target, and rejects missing, non-positive,
  active-task, or otherwise non-retryable targets without batch fallback;
  archived targets are also rejected unless exact retry supplies the explicit
  archived-recovery opt-in. Exact retry of a quarantined range additionally
  requires an explicit acknowledgement; without it the sticky quarantine
  state is preserved. Archived quarantine additionally requires exact
  `--include-archived`, but the pending command exposes that combination only
  as read-only dry-run validation. Neither acknowledgement widens active-task,
  terminal, or batch eligibility. An exact replay worker validates the profile
  and acquires the worker singleton before any write, then revalidates,
  requeues, and claims only that target in one transaction. It processes only
  the claimed task. Any non-successful exact attempt, including expired exact
  worker ownership after interruption, returns the task and range to archived
  quarantine rather than exposing default-profile work to a daemon.
- Doctor on a store with 1000 ordinary archived-history rows + 2 fresh failures
  reports the 2 actionable failures prominently, archived count secondary, and
  exits with the severity driven by the 2. Archived legacy rows that require
  exact recovery are separately reported as `admin-required`.
- Migration back-classifies existing failed extraction tasks, replay ranges,
  pending observations, and jobs (best-effort by error string, exhausted by
  default). Historical archived transient pending rows are the deliberate
  drain-only exception: they can re-enter only through the idle, 25-row,
  once-per-process or once-per-60-seconds bridge, with exponential backoff and
  current extraction priority. Non-archived transient legacy rows remain
  actionable until known-host drain success or explicit unknown-host repair;
  they are not silently archived by attempt count. Archived permanent or
  unknown-host rows use exact `recover-archived`.
- `docs/memory-lifecycle.md` or `docs/ARCHITECTURE.md` documents the failure
  lifecycle states.

## Risks

- Misclassification: on current recovery surfaces, a transient labeled
  permanent can archive something recoverable, mitigated by conservative
  mapping and explicit recovery tooling. The legacy pending bridge makes the
  safer opposite tradeoff: it never changes a replay failure to permanent, so
  a persistently bad row retries no faster than the 900-second cap and remains
  doctor-visible for `remem pending` repair.
- Retention window hides a recurring failure that re-fires after archiving:
  mitigated because each new occurrence is a fresh actionable row; only
  stale rows age out.
- A large historical pending backlog could create extraction work and AI cost:
  mitigated by current-work priority, a 25-row batch, once-only behavior for
  short-lived workers, a 60-second daemon interval, and exponential backoff.
- Restoring an archived transient row could blur audit state: mitigated by
  clearing failure/archive fields only in the same transaction that records
  its idempotent current-pipeline replacement; failed attempts preserve the
  source state.
- An operator could recover the wrong archived legacy row or assign the wrong
  host: mitigated by positive exact ID, mandatory dry-run guidance, rejection
  of non-failed/non-archived targets, and mandatory explicit host for stored
  unknown identity.
