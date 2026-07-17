# Failure Lifecycle Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #681
- Related: #426/#660 (extraction replay machinery), #374 (doctor probes), #365 (honest failure marking)

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
  bounded, logged retries — no manual `remem worker --once` babysitting.
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
- Worker logs every automatic retry with class, attempt number, and backoff;
  exhaustion and archiving transitions are logged at error/info respectively
  (U-29: no silent state changes).
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
  with backoff; attempts and class visible in logs and `remem status --json`.
  Legacy pending-observation failures are archive-only unless a production
  consumer is reintroduced.
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
  archived-recovery opt-in. Exact retry of a quarantined range additionally requires an
  explicit acknowledgement; without it the sticky quarantine state is
  preserved. Archived quarantine additionally requires exact
  `--include-archived`; the transaction clears only that target's archive
  marker. Neither acknowledgement widens active-task, terminal, or batch
  eligibility. An exact replay worker acquires the worker singleton before it
  revalidates/requeues the range, then claims and processes only the returned
  task with an explicitly validated profile, without a daemon race or sibling drain.
- Doctor on a store with 1000 archived + 2 fresh failures reports the 2
  actionable failures prominently, archived count secondary, and exits with
  the severity driven by the 2.
- Migration back-classifies existing failed extraction tasks, replay ranges,
  pending observations, and jobs (best-effort by error string, exhausted by
  default) so long-running installs converge without manual surgery or retry
  storms.
- `docs/memory-lifecycle.md` or `docs/ARCHITECTURE.md` documents the failure
  lifecycle states.

## Risks

- Misclassification: a permanent failure labeled transient wastes bounded
  retries (capped, acceptable); a transient labeled permanent archives
  something recoverable — mitigated by conservative mapping (unknown
  defaults to transient) and by archived rows remaining replayable via
  explicit `remem pending` tooling, including failed jobs.
- Retention window hides a recurring failure that re-fires after archiving:
  mitigated because each new occurrence is a fresh actionable row; only
  stale rows age out.
