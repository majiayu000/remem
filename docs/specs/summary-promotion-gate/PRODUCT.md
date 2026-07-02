# Summary Promotion Gate Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #674
- Evidence baseline: re-verification comment on #674 (2026-07-02)
- Downstream evidence consumers: #381, #383

## Problem

Every durable fact that arrives through the Stop/summary pipeline stalls in
`pending_review` forever, regardless of confidence, scope, or evidence. The
observation-extract path can auto-promote; the summary path cannot. This is a
whole input class silently excluded from promotion — the same failure shape as
the historical gate deadlocks (#238, #357).

Re-verification on 2026-07-02 found the exclusion is enforced twice:

1. The summary path passes no observation batch, and auto-promote only fires
   when a batch is present.
2. The summary path hardcodes `risk_class = "medium"` while the gate requires
   `low`. The risk check precedes the batch check, so the block reason recorded
   for summary candidates is always `risk_class_not_low`, hiding the first
   lock entirely.

Production impact on a live long-running install: promotion funnel
`Cand promoted: 3/1609 (0.2%)`, with `risk_class_not_low` blocking 1347
candidates (1067 in the trailing 7 days). This directly suppresses the
`memory_facts` growth evidence that #381/#383 acceptance requires, and the
asymmetry is invisible: doctor cannot distinguish "pending because low
confidence" from "pending because this path can never promote".

## Decision

Option A from #674, executed in two phases, with Option B's observability
landing first:

- Phase 1 (observability + shadow gate): candidates record which pipeline
  produced them; doctor and status split pending counts by source path; a
  summary-specific promotion gate runs in shadow mode and logs what it would
  promote without promoting anything.
- Phase 2 (enforcement): the summary gate goes live for a conservative
  allowlist of factual memory types, with thresholds chosen from Phase 1
  shadow telemetry rather than invented up front.

Rationale: the non-goal in #674 forbids bulk auto-approval of the existing
backlog without sampling evidence. Shadow mode produces that evidence on real
traffic before any behavior change.

## Goals

- Summary-path candidates with qualifying type, confidence, scope, and
  evidence can auto-promote, closing the summary half of the promotion funnel.
- Every summary candidate that does not promote carries an accurate,
  path-aware block reason; `risk_class_not_low` no longer masks the missing
  batch.
- Doctor and status report pending-review counts split by source path, so a
  stalled input class is visible within one release, not after months of
  backlog growth.
- Promotion decisions on the summary path are as observable as on the
  observation path: same logging shape, same block-reason vocabulary,
  extended where summary-specific.

## Non-Goals

- No relaxation of existing gate thresholds for the observation path.
- No bulk auto-approval of the existing `pending_review` backlog; existing
  rows keep their state and are only re-evaluated by explicit replay tooling.
- No change to the auto-promotable memory-type vocabulary
  (`lesson`/`preference` stay review-gated even when summary-derived).
- No LLM calls added to the promotion path.

## User-Visible Behavior

- `remem status` and `remem doctor` show candidate promotion counters split
  by source path (`observation` vs `summary`), each with block-reason
  breakdown and a trailing-7-day column.
- During Phase 1, doctor shows a `summary gate (shadow)` line reporting how
  many candidates would have promoted under the proposed thresholds.
- After Phase 2, summary-derived decisions and discoveries with qualifying
  confidence and evidence appear as active memories without manual review,
  and their promotion is logged with the gate verdict.

## Acceptance Criteria

- Fixture test: a qualifying summary-derived decision candidate auto-promotes
  in Phase 2 mode; the same fixture in Phase 1 mode stays pending and logs a
  shadow would-promote verdict.
- Fixture test: a summary candidate blocked by the gate records a
  summary-path block reason, never a masked `risk_class_not_low` produced by
  the hardcoded risk constant alone.
- Doctor output on a mixed store splits pending counts by source path.
- Post-change sampling on a real session shows summary-derived durable facts
  either promoting or visibly accounted for, feeding #381/#383 evidence.

## Risks

- Threshold too strict: shadow telemetry shows ~0 would-promote; the phase 2
  flip is then deferred and the confidence derivation for summary items is
  revisited instead of lowering the floor blindly.
- Threshold too loose: low-quality summary text becomes durable memory;
  bounded by the type allowlist, the unsafe-marker scan, and the existing
  suppression/invalidate lifecycle as the correction path.
