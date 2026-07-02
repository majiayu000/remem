# Summary Candidate Promotion Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #674
- Related: #381, #383 (production evidence chain), #238, #357 (historical gate deadlocks), #657/#658 (extraction backlog replay)

## Problem

Memory candidates created by the Stop/summary pipeline can never
auto-promote: the summary persistence path opts out of the auto-promote
evaluation entirely, so every durable fact arriving via summaries stalls in
`pending_review` regardless of confidence, scope, or evidence quality.

This is the same failure shape as two previously fixed gate deadlocks (#238
type-vocabulary mismatch, #357 near-zero support hit rate): an entire input
class silently excluded from promotion. It also suppresses exactly the
production evidence #381/#383 acceptance requires — growing `memory_facts`
counts from real sessions.

If the asymmetry is intentional (summary evidence is weaker than
observation-level evidence), it is currently undocumented and invisible:
nothing distinguishes "pending because low confidence" from "pending because
this path can never promote".

## Goals

- Make an explicit, recorded decision: wire summary candidates through the
  auto-promote gate, or keep the asymmetry and make it observable.
- Either way, end the silent stall: pending_review counts become explainable
  by source path.
- Feed the #381/#383 evidence chain: after this change, summary-derived
  durable facts either promote or are visibly accounted for.

## Non-Goals

- Relaxing existing gate thresholds for the observation-extract path.
- Bulk auto-approval of the existing pending_review backlog without sampling
  evidence.
- Changing what the summary pipeline extracts.

## Behavior Invariants

The decision between (A) and (B) is part of this spec's execution; the chosen
option's invariants apply. The default recommendation is (A) with a stricter
floor, because the gate already encodes every safety check the observation
path relies on.

Option A — wire summary candidates through the gate:

1. Summary-path candidates are evaluated by the same `should_auto_promote`
   logic as observation-path candidates, with a separate (stricter or equal)
   confidence floor for summary-sourced evidence, configurable and defaulting
   to at least the observation floor.
2. Gate blocks on the summary path log the same structured
   `auto_promote_block_reason` as the observation path.
3. No candidate promotes without evidence ids, exactly as today.

Option B — keep the asymmetry, make it observable:

1. The asymmetry is documented in `docs/ARCHITECTURE.md` with rationale.
2. Doctor/status splits pending_review counts by source path and labels the
   summary path "never auto-promotes (by design)".

Common:

4. The decision and rationale are recorded in this spec (updated in place)
   before implementation merges.
5. Post-change, a real-session sampling run is recorded on #674 showing
   summary-derived facts promoting (A) or visibly accounted for (B).

## Acceptance Criteria

- [ ] Decision recorded in this spec with rationale.
- [ ] If A: fixture test proves a qualifying summary candidate auto-promotes
      and a below-floor one blocks with a logged reason.
- [ ] If B: doctor splits pending_review by source path; ARCHITECTURE.md
      documents the asymmetry.
- [ ] The pinning test that currently asserts summary candidates never
      auto-promote is updated to assert the chosen contract instead.
- [ ] Real-session sampling evidence posted to #674 and cross-linked from
      #381/#383.

## Edge Cases

- Summary candidates whose supporting events were compressed or pruned before
  evaluation: gate must fail closed to pending_review (missing evidence never
  promotes), not error.
- Backlog: candidates inserted before the change keep their status; a separate
  replay decision (as with #657's `retry-extraction-ranges`) governs
  re-evaluation of the backlog, out of scope here.
- Hosts with summary-only capture (Codex without Bash observe): under (A)
  these hosts gain their first auto-promote path; sampling evidence must
  include at least one Codex session.

## Rollout Notes

Option A ships behind a config flag defaulting on only after the fixture suite
passes and one sampling window shows no bad promotions; the flag allows
instant reversion to today's behavior.
