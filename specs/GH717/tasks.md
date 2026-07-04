# Task Plan

## Linked Issue

GH-717

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [x] `SP717-T1` Owner: agent; Dependencies: GH-716 merged; Done when: GH-717 has a focused SpecRail packet that references the accepted GH-682 local semantic embedding contract; Verify: `python3 checks/check_workflow.py --repo . --spec-dir specs/GH717`.
- [x] `SP717-T2` Owner: agent; Dependencies: `SP717-T1`; Done when: observation dedup runs an active-provider vector stage after hash dedup, production extraction persistence calls it before insert, and provider-off behavior is preserved; Verify: `cargo test dedup -- --test-threads=1` and `cargo test observation_extract -- --test-threads=1`.
- [x] `SP717-T3` Owner: agent; Dependencies: `SP717-T1`; Done when: preference embedding fallback uses active-model embeddings and model-specific thresholds while preserving polarity/conflict guards; Verify: `cargo test preference -- --test-threads=1`.
- [x] `SP717-T4` Owner: agent; Dependencies: `SP717-T3`; Done when: curated-memory semantic dedup call sites remain on same-model active semantics for manual saves and candidate-promoted writes; Verify: `cargo test semantic_dedup -- --test-threads=1`.
- [x] `SP717-T5` Owner: agent; Dependencies: `SP717-T2` `SP717-T3` `SP717-T4`; Done when: version/doc/spec updates are synced and full verification passes; Verify: commands listed below.

## Parallelization

Implementation touches overlapping memory/dedup/preference paths, so keep the
writable implementation single-lane. Use a read-only reviewer lane before PR
gate/merge.

## Verification

- `cargo test dedup -- --test-threads=1`
- `cargo test observation_extract -- --test-threads=1`
- `cargo test semantic_dedup -- --test-threads=1`
- `cargo test preference -- --test-threads=1`
- `cargo fmt --check`
- `cargo check --message-format=short`
- `cargo test -- --test-threads=1`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH682`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH717`

## Handoff Notes

Do not close GH-682 automatically from the GH-717 PR. After merge, perform an
epic closure audit that all GH-682 phase PRs, eval evidence, and acceptance
criteria are linked and current.
