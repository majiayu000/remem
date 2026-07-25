# Task Plan

## Linked Issue

GH-676

## Implementation Issue

GH-694

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/associative-multihop-fixtures/PRODUCT.md` and
  `docs/specs/associative-multihop-fixtures/TECH.md`

## Tasks

- [x] `SP676-T1` Owner: agent; Dependencies: none; Done when: `specs/GH676` validates and GH-694 is linked as the implementation issue; Verify: `python3 checks/check_workflow.py --repo . --spec-dir specs/GH676`. Implemented by PR #695; GH-694 is closed and linked as the implementation issue.
- [x] `SP676-T2` Owner: agent; Dependencies: `SP676-T1`; Done when: `GoldenQuery` supports optional `hop_path` and associative fixtures require source/entity/target validation; Verify: golden validation tests. Implemented by PR #695 with loader validation for associative `hop_path`.
- [x] `SP676-T3` Owner: agent; Dependencies: `SP676-T2`; Done when: `eval/golden.json` contains at least 15 associative fixtures covering file path, crate, error signature, and issue-number entities; Verify: associative fixture contract test. Implemented by PR #695 with 15 associative fixtures across the required entity classes.
- [x] `SP676-T4` Owner: agent; Dependencies: `SP676-T2` `SP676-T3`; Done when: query-target lexical leakage is rejected mechanically; Verify: leaky associative fixture regression test. Implemented by PR #695 with deterministic query-target overlap rejection.
- [x] `SP676-T5` Owner: agent; Dependencies: `SP676-T3`; Done when: `remem eval-associative-baseline` generates the committed baseline/headroom report; Verify: CLI parse, command smoke, and checked-in report parity tests. Implemented by PR #695 with `eval/associative-multihop/baseline.json`.
- [x] `SP676-T6` Owner: agent; Dependencies: `SP676-T2` `SP676-T3` `SP676-T4` `SP676-T5`; Done when: local deterministic checks and focused Rust tests pass; Verify: commands below. Implemented by PR #695; this status-sync PR re-runs the SpecRail and focused associative checks.

## Parallel Split

No parallel writable lanes for this first slice. Schema, fixtures, validation,
and report generation all touch the same golden eval surface and should land
as one implementation PR.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH676`
- `cargo fmt --check`
- `cargo check --message-format=short`
- `cargo test associative`
- `cargo test cli_parses_eval_associative_baseline_options`
- `cargo test checked_in_golden_dataset_has_required_slices`
- `cargo test checked_in_golden_dataset_runs_against_fixture_corpus_without_live_db`
- `cargo test`

## Handoff Notes

PR #695 used `Refs #676` and `Closes #694` for this first implementation
slice. Do not close GH-676 until per-channel attribution, entity-BFS and
literal traversal deltas, trusted provenance fixture edge setup, and the ADR
decision follow-up land.
