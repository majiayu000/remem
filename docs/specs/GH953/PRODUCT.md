# Retrieval Engine Convergence — Product Spec

Issue: #953 (parent #942)
Status: Current contract (#953 closed after stage S1; deeper convergence remains future work)

## Closure status (2026-08-07)

#953 closed after the shared `SearchWeights` source and weighted fusion path
landed, and after the usage channel received its separately calibrated rollout.
The remaining graph-expansion delta was explicitly kept out of the
latency-sensitive SessionStart path until retrieval-side evidence justifies
enabling it there.

The broader convergence design below is retained as future guidance. It is not
completed work and is not an unfulfilled acceptance checklist for the closed
#953 slice. Continuing it requires a separately tracked implementation issue
and fresh evaluation evidence.

## Historical problem (before S1 and the #947 rollout)

Before S1, Remem answered the same question — "which memories matter for this
query?" — with two independently maintained implementations:

- `src/context/hybrid_context.rs`, used by SessionStart injection, which is what
  users actually receive.
- `src/retrieval/search/memory/`, used by `remem search` and the retrieval
  search/weight-grid evaluation harnesses.

Those paths had drifted. The search engine scored additional channels while
injection did not see `graph` or `usage` and did not apply
`min_evidence_confidence`. The tuning constants `RRF_K`,
`MAX_VECTOR_DISTANCE`, and the channel weights also existed twice, as private
`const` items in `hybrid_context.rs` and as fields of `SearchWeights`.

The consequence was that `eval-weight-grid` tuned `SearchWeights` while
injection did not read it, so **the evaluation harness optimized a path users
did not take**. S1 removed that weight-source duplication: injection now takes
an explicit `SearchWeights`, and its production caller uses
`SearchWeights::production()`. The later #947 rollout also added the usage
channel to injection with a calibrated default weight of `0.25` and
`REMEM_USAGE_WEIGHT=0` as the rollback. Shared channel assembly, graph parity,
and the evidence-confidence gate remain future convergence work.

## Future convergence goals

1. One engine computes candidate sets for both surfaces.
2. One source of truth for channel weights and fusion constants.
3. Callers express intent as a profile, not by re-implementing channel assembly.
4. Evaluation results provably describe the injection path.

## Non-Goals

- Changing default retrieval quality. This change is behavior-preserving for
  injection except where the issue explicitly calls for the missing channels;
  any ranking delta must be justified by evaluation, not by the refactor.
- Enabling the `usage` channel was outside S1 and belonged to #947. #947 later
  shipped it separately at the calibrated `0.25` default with an operator
  rollback to `0`; this historical staging boundary is not a statement of the
  current runtime default.
- Re-tuning weights. Producing new tuned values is downstream work.

Issue #954 is a narrow correctness follow-up rather than a #953 convergence
stage: rank-only channels must contribute pure weighted RRF instead of receiving
a second synthetic `1 / rank` signal. Because that correction changes injection
ordering, `eval-injection` runs the old and new rank-signal behavior through the
same injection channel assembly and requires non-regressing MRR@10 and nDCG@10.
It does not introduce the shared engine, enable missing channels, or change the
remaining #953 stages.

## Success Criteria

- A test asserts both surfaces derive their channel set and weights from the
  same `SearchWeights` value; adding a channel to the engine without updating a
  profile fails to compile or fails that test.
- Injection evaluation shows no regression against the pre-change baseline.
- A weight-grid change is demonstrably reflected in injection output, proven by
  a test that varies `SearchWeights` and observes injection ordering change.
- No `const` duplicating a `SearchWeights` field remains in `hybrid_context.rs`.

S1 establishes only the shared weight-source prerequisite. It does not make the
search evaluator execute the injection path, apply a generated weight-grid
report at runtime, or satisfy the broader shared-channel and injection-evaluation
criteria. Those remain future convergence work rather than closure blockers for
the narrowed #953 slice.
