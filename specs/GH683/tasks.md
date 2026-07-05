# Task Plan

## Linked Issue

GH-683

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/review-queue-throughput/PRODUCT.md` and
  `docs/specs/review-queue-throughput/TECH.md`

## Implementation Tasks

- [ ] `SP683-T1` Owner: implementation agent; Dependencies: none; Done when: `memory_candidates` has nullable review metadata columns and an index for queue-age reads; Verify: migration-backed candidate review tests.
- [ ] `SP683-T2` Owner: implementation agent; Dependencies: `SP683-T1`; Done when: shared review queue stats expose pending totals, age metrics, inflow/resolved counts, project splits, and block-reason examples; Verify: `cargo test memory_candidate::review_stats -- --nocapture`.
- [ ] `SP683-T3` Owner: implementation agent; Dependencies: `SP683-T2`; Done when: `remem status --json` and doctor include review queue health and deadlock warnings; Verify: status and `doctor::review_queue` tests.
- [ ] `SP683-T4` Owner: implementation agent; Dependencies: `SP683-T1`; Done when: `approve-batch`, `discard-batch`, and `blocked` commands support accepted filters, preview/confirm, default cap, transactional mutation, and durable review provenance; Verify: `cargo test memory_candidate::review -- --nocapture`.
- [ ] `SP683-T5` Owner: implementation agent; Dependencies: `SP683-T2` `SP683-T4`; Done when: REST candidate list accepts matching filters and exposes block-reason aggregates; Verify: focused API regression tests.
- [ ] `SP683-T6` Owner: implementation agent; Dependencies: `SP683-T1` `SP683-T2` `SP683-T3` `SP683-T4` `SP683-T5`; Done when: local deterministic checks and full Rust tests pass and the implementation PR closes GH-683; Verify: commands below and PR gate evidence.

## Parallelization

No parallel writable lanes. Schema, candidate review helpers, status, doctor,
CLI, and REST all share the candidate review contract and should land in one
coherent implementation PR.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH683`
- `python3 checks/route_gate.py --repo . --route implement --issue 683 --state ready_to_implement --json`
- `cargo fmt --check`
- `cargo check`
- `cargo test memory_candidate::review -- --nocapture`
- `cargo test memory_candidate::review_stats -- --nocapture`
- `cargo test doctor::review_queue -- --nocapture`
- `cargo test cli::actions::query::status -- --nocapture`
- `cargo test`

## Handoff Notes

Use `Closes #683` in the implementation PR after all acceptance criteria and
tests land. Do not change the auto-promotion predicate as part of this issue.
