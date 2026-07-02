# Cache-Stable Injection Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #673
- Related: #205 (Codex per-conversation gating, closed), #384 (per-session cost visibility)

## Problem

Prompt prefix caching bills cache reads at roughly one tenth of input price,
and any upstream byte change invalidates everything after it. remem's injected
block sits near the top of every session's context, so byte churn in that
block silently multiplies the cost of every remem-enabled session ("Don't
Break the Cache", arXiv 2601.06007, documents 85-95% cost savings on hits for
long-horizon agents).

remem's injection gate already suppresses re-injecting identical content, but
there is no byte-stability contract for the block itself: relative timestamps
("updated 3:36pm"), volatile counters, and score-dependent ordering can change
the rendered bytes between sessions whose underlying memory set is identical.

## Goals

- Make the rendered context block a deterministic function of the memory
  state: unchanged state produces byte-identical output.
- Keep the standing block stable within a session: mid-session additions are
  appended after the stable prefix, never edited into it.
- Measure block churn as an eval metric so regressions are caught in CI.

## Non-Goals

- Changing which memories are selected (ranking and budgets untouched).
- Cross-host or cross-session cache coordination.
- Guaranteeing host-side cache hits (hosts own the actual cache); remem's
  contract is to stop being the reason for misses.

## Behavior Invariants

1. Two consecutive renders against an unchanged database produce
   byte-identical blocks.
2. Rendered content contains no wall-clock-relative values (relative times,
   "now" deltas) and no run-local counters inside the stable prefix; absolute
   epochs are allowed because they are state, not run artifacts.
3. Section order and within-section ordering are deterministic: ties in score
   break by stable keys (memory id), never by iteration order.
4. Adding one memory changes only the sections that logically contain it; the
   bytes before the first affected section are unchanged.
5. UserPromptSubmit-time injections are additive blocks placed after the
   SessionStart prefix and never rewrite it.
6. The determinism contract is versioned: intentional renderer changes bump a
   render-contract version recorded in eval output, so churn regressions are
   distinguishable from intentional format changes.

## Acceptance Criteria

- [ ] Determinism test: render twice on a fixture DB, assert byte equality;
      runs in CI.
- [ ] Volatile-field audit: no relative timestamps or run-local counters in
      the stable prefix, enforced by a renderer unit test on fixture output.
- [ ] Churn metric: eval reports bytes-changed between renders for (a)
      unchanged DB (must be 0) and (b) one-memory-added (reported, with the
      unchanged-prefix property asserted).
- [ ] Render-contract version appears in eval JSON output.

## Edge Cases

- Memories whose scores legitimately change between sessions (age decay):
  decay affects selection, and selection changes are real state changes; the
  invariant only requires byte identity for identical selection inputs, so
  decay-driven reordering must be quantized (bucketed ages) or excluded from
  ordering keys to avoid churn without state change.
- Truncation boundaries: budget truncation must cut at deterministic points
  (item boundaries with stable keys), never mid-item at a byte offset that
  depends on incidental content length elsewhere.
- Hosts that re-render per prompt (UserPromptSubmit retrieval injection): the
  additive block may vary per prompt by design; only the SessionStart prefix
  carries the stability contract.

## Rollout Notes

Pure renderer contract change plus eval additions; no schema migration. Ship
behind no flag — determinism is not user-visible behavior change — but call
out in the changelog that the block layout was normalized (screenshots and
downstream parsers of the old layout may notice cosmetic diffs).
