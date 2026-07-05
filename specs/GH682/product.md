# Product Spec

## Linked Issue

GH-682

## Implementation Issues

- GH-714: Phase 1 provider contract and degraded-state visibility
- GH-715: Phase 2 local semantic ONNX model, same-model vectors, and backfill
- GH-716: Phase 3 provider comparison eval gate and default-flip evidence
- GH-717: Phase 4 downstream semantic dedup and preference adoption

## Accepted Contract

The authoritative product contract is
`docs/specs/local-semantic-embedding/PRODUCT.md`.

This SpecRail packet hands the accepted #682 contract to the implementation
issues above. It does not replace the `docs/specs/` contract and it does not
close GH-682 by itself.

## User Problem

The vector channel has the highest hybrid retrieval weight, but default
installs currently use `remem-local-feature-hash-v1`, a deterministic
bag-of-features fallback rather than a learned semantic model. Paraphrase and
synonym recall therefore fail for users without an embedding API key, and
dedup/preference consolidation inherit the same non-semantic ceiling.

## Goals

- Make embedding provider selection explicit and visible through config,
  status, and doctor output.
- Ship a real local semantic embedding path with CJK+English coverage, using
  downloaded model weights rather than bundled release assets.
- Prevent mixed-model cosine scoring and provide an explicit backfill path
  when providers change.
- Require committed eval evidence before changing the default provider.
- Move downstream dedup and preference consolidation onto the active semantic
  space only after the provider and eval gates are in place.

## Non-Goals

- Do not add ANN indexing in this epic.
- Do not remove the feature-hash fallback.
- Do not change fusion weights without eval evidence.
- Do not close GH-682 from any single phase unless all epic acceptance
  criteria are verified.

## Behavior Invariants

1. Provider resolution is explainable: users can see the configured provider,
   active provider, active model id, disabled/degraded state, and active-model
   vector coverage.
2. Fallback is never silent: if the active provider differs from the configured
   provider, status/doctor and logs expose that state.
3. `provider = "off"` disables the vector channel explicitly and does not warn
   as degraded.
4. Local semantic weights are stored under the remem data directory and are
   never bundled into the binary.
5. Search compares vectors only when query and memory vectors share the same
   model id.
6. Provider changes offer an idempotent backfill path with coverage reporting
   before any pruning of older vectors.
7. Default-provider changes require committed feature-hash, local semantic,
   and API comparison evidence under `eval/`.
8. Downstream semantic dedup and preference consolidation use thresholds
   calibrated for the active model id and preserve conflict/polarity guards.

## Acceptance Criteria

- [x] GH-714 lands the provider config, `status`, and `doctor` visibility
      slice. Evidence: PR #719 closed GH-714 on 2026-07-04.
- [x] GH-715 lands local semantic model download/runtime support, multi-model
      vector storage, same-model cosine filtering, and backfill. Evidence:
      PR #728 closed GH-729, the GH-715 multi-model storage slice, on
      2026-07-04; PR #731 closed GH-715 on 2026-07-04.
- [x] GH-716 commits provider comparison eval evidence and records the default
      provider decision before any default flip. Evidence:
      PRs #732 and #733 closed GH-716 on 2026-07-04;
      `eval/provider-comparison/report.json`; decision: keep the default
      unchanged until local/API comparison rows are available.
- [x] GH-717 updates downstream semantic dedup and preference consolidation
      after the eval gate. Evidence: PRs #734 and #735 closed GH-717 on
      2026-07-04.
- [x] GH-682 is closed only after all phase issues and the eval evidence are
      verified. Evidence: GH-714, GH-715, GH-716, and GH-717 are closed; the
      docs/spec index records the no-default-flip decision and downstream
      adoption state.

## Edge Cases

- API provider configured without a key falls back only when a fallback is
  configured and must report degraded state.
- Missing local model files in hook paths defer embedding work rather than
  blocking on download or writing feature-hash vectors as if they were local
  semantic vectors.
- Existing feature-hash vectors remain valid under their model id and are
  ignored by active semantic queries until explicitly selected or pruned.
- Unsupported platforms or ONNX runtime failures keep search honest by
  reporting provider readiness and degraded behavior.

## Rollout Notes

Ship the phases in order. Use `Refs #682` on phase PRs and close only the
focused implementation issue for that phase. GH-682 remains the epic until the
phase issues, eval artifacts, docs/spec index decision, and downstream adoption
are complete.

## Closure Audit

GH-682 has satisfied its implementation closure criteria because all four
implementation issues are closed by merged PRs:

- GH-714: PR #719, provider contract and degraded-state visibility.
- GH-715: PR #728 / GH-729 for multi-model storage, plus PR #731 for local
  semantic runtime, same-model vectors, and backfill.
- GH-716: PRs #732 and #733, provider comparison eval evidence and no-flip
  default decision.
- GH-717: PRs #734 and #735, downstream active-model dedup and preference
  adoption.
