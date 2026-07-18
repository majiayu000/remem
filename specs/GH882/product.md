# Product Spec

## Linked Issue

GH-882

## User Problem

Memory-candidate extraction can permanently quarantine an otherwise valid
rollup when the LLM emits the intuitive but unsupported type `fact`. One
out-of-vocabulary type currently invalidates the entire extraction task, so
users lose all candidate memories from the affected event range after retries
are exhausted.

## Goals

- Accept `fact` as a recoverable memory-type alias and preserve the rest of the
  extraction result.
- Normalize `fact` to the existing durable-memory type `discovery`.
- Make the extraction contract explicitly enumerate the supported candidate
  memory types so future model output is less likely to use observation or
  intuitive type names.

## Non-Goals

- Recovering or replaying already quarantined production tasks.
- Changing the canonical memory-candidate type vocabulary.
- Handling summary-job frozen writes, rollup redaction, or replay behavior
  tracked by GH-684 and GH-864.
- Guessing a normalization for arbitrary unknown memory types.

## Behavior Invariants

1. A candidate whose type is `fact` is accepted and represented as a
   `discovery` candidate.
2. A supported canonical candidate type keeps its existing meaning.
3. Existing observation-type aliases keep their existing normalization.
4. An unknown type other than an explicitly supported alias remains a
   malformed-output error; the pipeline does not silently coerce arbitrary
   values.
5. The memory-candidate extraction instructions enumerate all canonical
   candidate types and tell the model to use `discovery` for factual findings.
6. Parsing `fact` cannot discard other valid candidates from the same model
   response.

## Acceptance Criteria

- [ ] `normalize_memory_type("fact")` returns `discovery`.
- [ ] Canonical types and existing observation aliases continue to pass their
  regression tests.
- [ ] A still-unknown type continues to return an explicit error.
- [ ] The extraction prompt names the seven canonical candidate types and
  directs factual findings to `discovery`.
- [ ] Focused tests, workflow validation, and the repository's required Rust
  verification pass.

## Edge Cases

- Matching follows the parser's existing whitespace and case normalization.
- Near-miss or invented types are not accepted unless separately specified.
- Responses containing multiple candidates remain atomic under the existing
  parser contract; this change only prevents `fact` from being the malformed
  member.

## Rollout Notes

This is backward-compatible normalization and prompt clarification. It needs no
schema migration or data rewrite. Operators may retry quarantined extraction
ranges separately after deploying the fix; that operational replay is outside
this issue.
