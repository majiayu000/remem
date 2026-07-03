# Task Plan

## Linked Issue

GH-682

## Implementation Issues

- GH-714: Phase 1 provider contract and degraded-state visibility
- GH-715: Phase 2 local semantic ONNX model, same-model vectors, and backfill
- GH-716: Phase 3 provider comparison eval gate and default-flip evidence
- GH-717: Phase 4 downstream semantic dedup and preference adoption

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/local-semantic-embedding/PRODUCT.md` and
  `docs/specs/local-semantic-embedding/TECH.md`

## Implementation Tasks

- [ ] `SP682-T1` Owner: agent; Dependencies: none; Done when: `specs/GH682` validates and GH-714, GH-715, GH-716, and GH-717 are linked as implementation issues; Verify: `python3 checks/check_workflow.py --repo . --spec-dir specs/GH682`.
- [ ] `SP682-T2` Owner: agent; Dependencies: spec approval; Done when: GH-714 lands provider config parsing, fallback resolution, `off` behavior, status/doctor visibility, active-model coverage, and error-level degraded fallback logging; Verify: config, status, doctor, and embedding focused tests.
- [ ] `SP682-T3` Owner: agent; Dependencies: `SP682-T2`; Done when: GH-715 lands local semantic model download/status, model-dir/checksum handling, hook-safe readiness behavior, multi-model vector storage, same-model cosine filtering, idempotent backfill, and explicit prune gating; Verify: embedding, vector, migration, and backfill focused tests.
- [ ] `SP682-T4` Owner: agent; Dependencies: `SP682-T3`; Done when: GH-716 commits provider comparison eval reports for feature-hash, local semantic, and API embeddings, records default-flip criteria, and updates the #682 evidence trail before any default change; Verify: eval provider comparison, `cargo run -- eval-extraction --json --check-baseline`, and `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`.
- [ ] `SP682-T5` Owner: agent; Dependencies: `SP682-T4`; Done when: GH-717 moves memory dedup, curated-memory semantic dedup, and preference consolidation onto active-model semantics with calibrated thresholds and polarity/conflict guards; Verify: dedup, semantic_dedup, and preference focused tests.
- [ ] `SP682-T6` Owner: agent; Dependencies: `SP682-T2` `SP682-T3` `SP682-T4` `SP682-T5`; Done when: GH-682 has links to all phase PRs, eval evidence under `eval/`, updated docs/spec index decision, and all acceptance criteria verified; Verify: `cargo fmt --check`, `cargo check`, `cargo test`, eval commands, and final issue audit.

## Parallelization

Do not run the provider/runtime phases as parallel writable lanes. GH-714 must
land before GH-715 because the runtime depends on the provider-state contract.
GH-716 depends on GH-715 because it compares the real local model. GH-717
depends on GH-716 because dedup thresholds must be calibrated against the
chosen active model id.

Read-only review lanes may inspect the accepted docs contract and child issue
scope in parallel.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH682`
- `python3 checks/route_gate.py --repo . --route write_spec --issue 682 --state ready_to_spec --json`
- Phase PRs must additionally run:
  - `cargo fmt --check`
  - `cargo check --message-format=short`
  - Focused tests for touched embedding, vector, eval, dedup, preference,
    status, and doctor surfaces
  - `cargo test` before merge readiness

## Handoff Notes

Use `Refs #682` in every phase PR. Close only the focused implementation issue
for that phase, such as `Closes #714`, when its acceptance criteria and tests
land. Do not close GH-682 until all four implementation issues, eval evidence,
and downstream adoption are complete and verified.
