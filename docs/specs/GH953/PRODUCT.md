# Retrieval Engine Convergence — Product Spec

Issue: #953 (parent #942)
Status: Current contract (stage S1 implemented; issue remains open)

## Problem

Remem answers the same question — "which memories matter for this query?" — with
two independently maintained implementations:

- `src/context/hybrid_context.rs`, used by SessionStart injection, which is what
  users actually receive.
- `src/retrieval/search/memory/`, used by `remem search` and the retrieval
  search/weight-grid evaluation harnesses.

They have drifted. The search engine scores seven channels; injection scores
five. Injection never sees the `graph` channel (weight 0.75) or the `usage`
channel, and never applies `min_evidence_confidence`. The tuning constants
`RRF_K`, `MAX_VECTOR_DISTANCE`, and the six channel weights exist twice, as
private `const` items in `hybrid_context.rs` and as fields of `SearchWeights`.

The consequence is that `eval-weight-grid` tunes `SearchWeights`, injection does
not read `SearchWeights`, and so **the evaluation harness optimizes a path users
do not take**. Every reported retrieval gain is unproven for the product's
primary surface, and any future weight change silently applies to one path only.

## Goals

1. One engine computes candidate sets for both surfaces.
2. One source of truth for channel weights and fusion constants.
3. Callers express intent as a profile, not by re-implementing channel assembly.
4. Evaluation results provably describe the injection path.

## Non-Goals

- Changing default retrieval quality. This change is behavior-preserving for
  injection except where the issue explicitly calls for the missing channels;
  any ranking delta must be justified by evaluation, not by the refactor.
- Enabling the `usage` channel by default. That is #947 and stays gated at
  `usage = 0.0` here.
- Re-tuning weights. Producing new tuned values is downstream work.

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
report at runtime, or satisfy the shared-channel and injection-evaluation
criteria. Those remain required before #953 can close.
