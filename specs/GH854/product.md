# Product Spec

## Linked Issue

GH-854

complexity: large

status: approved

## User Problem

SessionStart currently fills the available character and section budgets with
Lessons, MemoryIndex, and Sessions even when those items have little connection
to the current task. Users can see when the final context was truncated, but
they cannot see when non-Core items were omitted for low relevance or because
the selected injection count was reached.

The maintainer approved an evidence charter on 2026-07-19: use the existing
golden evaluator at `k in {1,3,5,10}`, the existing capacity metrics, and the
existing injection evaluator's rendered character count. Select the smallest
`k` whose required per-slice `hit_at_k` is within one percentage point of the
best arm. The approved charter replaces the earlier draft's proposed
coding-bench and signed-tag requirements.

## Goals

- Prefer a small set of task-relevant non-Core SessionStart items over filling
  the existing budget with weakly related content.
- Reuse the existing local significant-token relevance behavior; do not add a
  network, LLM, or second injection gate.
- Select the default non-Core injection count from a reproducible four-arm
  sweep instead of intuition.
- Make low-relevance blanking, k limiting, existing section limits, and final
  character truncation visible in the SessionStart footer and `remem status`
  text/JSON.

## Non-Goals

- Changing Core selection, Preferences, Workstreams, host parity, capture,
  extraction, retrieval storage, or database schema.
- Replacing the hybrid retriever or implementing a global reranker.
- Introducing coding-bench orchestration, signed approval tags, trust roots, or
  another evaluation/gating framework.
- Treating a larger `k` as better or worse before the sweep is measured.

## Behavior Invariants

1. `P-001` Core, Preferences, Workstreams, host behavior, and the existing
   section/total character budgets retain their current selection and rendering
   semantics.
2. `P-002` Lessons, the MemoryIndex view after Core IDs are excluded, and
   Sessions use the same local significant-token relevance score before their
   existing section limits are applied.
3. `P-003` A candidate with zero significant-token overlap is never injected to
   fill a slot. If no governed candidate is relevant, the three governed
   sections may be blank while non-governed sections continue normally.
4. `P-004` Positive candidates are ordered by descending score, then the stable
   section order Lessons, MemoryIndex, Sessions, then stable item identity. The
   result is deterministic for the same query, candidate snapshot, and policy.
5. `P-005` One global non-Core `k` applies across the three governed sections.
   Core items do not consume k. Eligible candidates outside k are omitted, and
   fewer than k positive candidates are not backfilled.
6. `P-006` The score is the fraction of significant query tokens found in the
   candidate text, reusing the existing PromptSubmit tokenizer and stop-token
   rules. The per-request threshold is derived from the selected k: use the
   midpoint between the kth score and the next lower positive score when a gap
   exists; otherwise use the kth score. Stable tie-breaking enforces k. With
   fewer than k positive candidates, use the lowest positive score. With no
   positive score, the governed selection is blank.
7. `P-007` `REMEM_CONTEXT_RELEVANCE_K=0` restores the legacy governed-section
   selection as a rollback switch. A positive override changes k without
   changing the scorer. The evidence-selected value is the default.
8. `P-008` The SessionStart footer distinguishes policy disabled, applied, and
   blank states and reports k, threshold, candidate/eligible/final injected
   counts, low-relevance drops, k drops, section drops, and total truncation.
9. `P-009` The latest-session block in `remem status` and `remem status --json`
   exposes the most recent available relevance policy state and audit counts
   from `context_injection_items`. Legacy databases or runs without policy
   evidence report unavailable rather than claiming relevance was applied.
10. `P-010` The committed sweep report contains the exact commands, dataset and
    source identity, `k={1,3,5,10}`, required per-slice and overall golden
    metrics, representative SessionStart output characters, existing capacity
    degradation metrics, and the deterministic recommendation.
11. `P-011` An arm is eligible only when every populated golden slice's
    `hit_at_k` is within `0.01` of that slice's best measured value. The
    smallest eligible arm is selected. Ties choose the smaller k. Missing arms
    or missing required slice data produce no recommendation.
12. `P-012` The implementation records closed audit reasons for
    `below_sessionstart_relevance_threshold`, `sessionstart_k_limit`,
    `section_budget`, and existing final gate/truncation outcomes without
    storing query or memory text in new diagnostics.

## Acceptance Criteria

- [ ] Four reproducible `remem eval --json` runs at k 1, 3, 5, and 10 are
      committed as a summarized report with raw evidence hashes.
- [ ] The report applies the all-populated-slices one-percentage-point rule and
      records the selected default k without hiding secondary metric tradeoffs.
- [ ] Low-relevance Lessons, MemoryIndex items, and Sessions do not backfill,
      while Core, Preferences, and Workstreams are unchanged.
- [ ] Selection, threshold derivation, ties, sparse candidates, no-positive
      candidates, and the k=0 rollback path have focused tests.
- [ ] SessionStart and `remem status` text/JSON distinguish relevance blanking
      from k/section/total budget effects.
- [ ] Existing capacity and eval gates pass, and the report includes their
      degradation and SessionStart character evidence.

## Edge Cases

- Empty or weak task signals: governed sections are blank and visible; other
  sections still render.
- Tied kth score: the threshold includes the tie, while stable identity order
  enforces exactly k.
- Fewer than k positive candidates: inject only those candidates.
- Invalid environment value: preserve the existing env parsing convention and
  use the evidence-selected default.
- Legacy audit rows: status reports relevance evidence as unavailable.
- Final character truncation: remains distinct from relevance and k drops.

## Release Notes

This is a default behavior change with an explicit `k=0` rollback switch. The
release note and README must name the governed sections, selected k, relevance
method, status visibility, privacy boundary, and rollback environment variable.
