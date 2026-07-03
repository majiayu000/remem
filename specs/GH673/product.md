# Product Spec

## Linked Issue

GH-673

## Accepted Contract

The authoritative product contract is
`docs/specs/cache-stable-injection/PRODUCT.md`.

This SpecRail packet hands the existing #673 contract to workflow tracking. It
does not replace the `docs/specs/` contract and does not approve runtime
implementation by itself.

## User Problem

remem injects a context block near the top of every Claude Code and Codex
session. Anthropic-style prefix caches are byte-sensitive, so relative
timestamps, unstable ordering, volatile counters, or mid-session rewrites in
that block can invalidate the cache even when the underlying memory state did
not change. The result is avoidable cost and latency for users who keep remem
enabled across many agent sessions.

## Goals

- Make SessionStart context rendering byte-stable for unchanged memory state.
- Keep the standing SessionStart prefix stable within a session.
- Ensure prompt-time additions are additive and placed after the stable prefix.
- Add deterministic churn reporting so CI can catch renderer regressions.
- Version the render contract so intentional format changes are auditable.

## Non-Goals

- Do not change which memories are selected or ranked.
- Do not coordinate prompt caches across hosts.
- Do not guarantee provider-side cache hits; remem only guarantees stable
  bytes for its own injected prefix.
- Do not introduce a schema migration for this renderer-focused change.

## Behavior Invariants

1. P1: Two consecutive SessionStart renders against unchanged memory state
   produce byte-identical output.
2. P2: The stable prefix contains no wall-clock-relative values, run-local
   counters, or other data that changes without a memory-state change.
3. P3: Section order and within-section order are total and deterministic,
   including stable tie-breaks for equal scores.
4. P4: Adding one memory changes only the section that logically contains it;
   bytes before the first affected section remain unchanged.
5. P5: UserPromptSubmit or prompt-time retrieval additions are appended after
   the SessionStart prefix and never rewrite the standing prefix.
6. P6: Eval JSON includes a render-contract version so intentional renderer
   changes can be distinguished from accidental byte churn.

## Acceptance Criteria

- [ ] Two consecutive `remem context` renders on an unchanged fixture database
      produce identical bytes in CI.
- [ ] Renderer tests reject relative timestamps and run-local counters inside
      the stable prefix.
- [ ] Equal-score or equal-priority items are ordered by stable keys, not
      iteration order.
- [ ] The one-memory-added churn eval reports changed-byte count and asserts
      unchanged-prefix preservation before the first affected section.
- [ ] Prompt-time/additional-context injection tests prove the SessionStart
      prefix bytes are unchanged after additive injection.
- [ ] Eval JSON includes `render_contract_version`.

## Edge Cases

- Score or staleness changes caused by age decay must not reorder unchanged
  selection inputs; ordering should use stable buckets or resolved labels.
- Truncation must drop whole items at deterministic boundaries, not cut at
  incidental byte offsets.
- Empty sections must render consistently and must not include run-specific
  counts.
- If the renderer format intentionally changes, the render-contract version
  must change with it.
- Prompt-time retrieval blocks may vary per prompt, but that variability must
  stay outside the SessionStart stable prefix.

## Rollout Notes

Implementation is a renderer and eval contract change with no data migration.
Release notes should mention that the context block layout was normalized for
prompt-cache stability because snapshot tests or external parsers may notice
cosmetic output differences.
