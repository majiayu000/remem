# Task Plan

## Linked Issue

GH-671

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/preference-rule-compilation/PRODUCT.md` and
  `docs/specs/preference-rule-compilation/TECH.md`

## Implementation Tasks

- [x] `SP671-T1` Owner: agent; Dependencies: spec approval; Done when: config defaults, schema migration, preference reinforcement state or equivalent typed preference metadata, rule override state, and diagnostic state exist without enabling runtime behavior by default; Verify: migration/schema tests and focused config parser tests. Implemented in partial PR with `v062_preference_rule_state`, disabled-by-default config, schema drift/convergence guardrails, and focused tests.
- [x] `SP671-T2` Owner: agent; Dependencies: `SP671-T1`; Done when: versioned artifact structs, closed predicate enum, pure evaluator, corrupt/missing artifact fail-open behavior, and atomic artifact writer are implemented; Verify: `cargo test rules -- --nocapture` covers artifact schema round-trip, unsupported predicate parse failure, deterministic evaluation, disabled-rule skipping, invalid-regex fail-open behavior, missing/corrupt/wrong-version artifact fail-open behavior, stable artifact paths, and atomic-write preservation on injected rename failure.
- [ ] `SP671-T3` Owner: agent; Dependencies: `SP671-T1` `SP671-T2`; Done when: the compiler selects only eligible active preferences from canonical preference reinforcement state, merges user overrides, removes superseded/suppressed/expired/deleted source rules, resolves conflicts by newest authoritative source, and writes the project artifact only from the worker; Verify: compiler eligibility, lifecycle removal, conflict, worker-only artifact write, and override-merge tests.
- [ ] `SP671-T4` Owner: agent; Dependencies: `SP671-T1` `SP671-T3`; Done when: `remem rules list`, `disable`, `enable`, and `set-action warn|block` expose provenance and persist overrides through artifact deletion and recompile; Verify: CLI round-trip tests.
- [ ] `SP671-T5` Owner: agent; Dependencies: `SP671-T2` `SP671-T3`; Done when: Claude Code install/dispatch supports PreToolUse Bash evaluation before command execution, PostToolUse remains capture-only, and Codex block-mode enforcement is rejected as unsupported; Verify: simulated hook integration tests for warn, block, capture-only, and unsupported-host paths.
- [ ] `SP671-T6` Owner: agent; Dependencies: `SP671-T1` `SP671-T3` `SP671-T5`; Done when: `remem doctor` reports rule count, artifact presence, last compile time, last compile/evaluation error, and per-host enforcement capability without printing rule payload secrets; Verify: doctor human and JSON tests.
- [ ] `SP671-T7` Owner: agent; Dependencies: `SP671-T3` `SP671-T5`; Done when: repeated-correction fixtures cover package-manager choice, forbidden commit trailers, and forbidden commands, and the hook latency benchmark shows p95 unchanged within measurement noise; Verify: fixture/eval commands and latency benchmark output.
- [ ] `SP671-T8` Owner: agent; Dependencies: `SP671-T1` `SP671-T2` `SP671-T3` `SP671-T4` `SP671-T5` `SP671-T6` `SP671-T7`; Done when: docs and architecture notes reflect the shipped behavior, all acceptance criteria are checked, and #671 is updated with implementation evidence; Verify: `cargo fmt --check`, `cargo check`, focused tests, `cargo test`, `python3 checks/check_workflow.py --repo . --spec-dir specs/GH671`, and `git diff --check`.

## Parallelization

Most work should stay serial through `SP671-T3` because migration, artifact
schema, compiler, and evaluator contracts overlap. After `SP671-T3`, CLI
(`SP671-T4`), doctor (`SP671-T6`), and docs portions can proceed in parallel
if writable file ownership is split explicitly.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH671`
- `python3 checks/route_gate.py --repo . --route write_spec --issue 671 --state ready_to_spec --json`
- `python3 checks/route_gate.py --repo . --route implement --issue 671 --state ready_to_implement --json`
- `cargo fmt --check`
- `cargo check`
- Focused preference reinforcement, rule compiler, evaluator, CLI, hook,
  doctor, fixture, and latency tests
- `cargo test` before merge readiness

## Handoff Notes

Use `Refs #671` for spec and partial implementation PRs. Do not close #671
until every acceptance criterion in `product.md` and the authoritative
`docs/specs/preference-rule-compilation/PRODUCT.md` contract is implemented
and verified. Human spec approval and the `ready_to_implement` label are still
required before runtime implementation.
