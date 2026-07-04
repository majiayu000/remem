# Task Plan

## Linked Issue

GH-716

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [x] `SP716-T1` Owner: agent; Dependencies: GH-715 merged; Done when: `eval/golden.json` has English and CJK provider-comparison paraphrase fixtures with stable slice metadata; Verify: golden fixture validation tests.
- [x] `SP716-T2` Owner: agent; Dependencies: `SP716-T1`; Done when: a provider-comparison eval command/report builder emits rows for `feature-hash`, `local`, and `api` with honest unavailable states and baseline metrics; Verify: focused provider-comparison tests.
- [x] `SP716-T3` Owner: agent; Dependencies: `SP716-T2`; Done when: default flip criteria and decision are written into the report and docs/spec index; Verify: committed report inspection plus workflow checks.
- [x] `SP716-T4` Owner: agent; Dependencies: `SP716-T1` `SP716-T2` `SP716-T3`; Done when: full required verification passes and PR closes GH-716 without closing GH-682; Verify: commands listed below.

## Parallelization

The fixture/design work and report-builder implementation touch overlapping
eval files, so keep implementation single-lane. A read-only reviewer lane can
inspect the final PR after tests pass.

## Verification

- `cargo test eval`
- `cargo run -- eval-extraction --json --check-baseline`
- `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`
- provider-comparison command/report added by this phase
- `cargo fmt --check`
- `cargo check --message-format=short`
- `cargo test`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH682`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH716`

## Handoff Notes

Do not implement GH-717 in this branch. GH-717 depends on this report's default
decision and threshold evidence. If local/API providers are unavailable in CI,
the correct GH-716 outcome is a committed no-flip decision with blockers, not a
silent fallback or fabricated pass.
