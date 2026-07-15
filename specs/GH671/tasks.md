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
- [x] `SP671-T3` Owner: agent; Dependencies: `SP671-T1` `SP671-T2`; Done when: the compiler selects only eligible active preferences from canonical preference reinforcement state, merges user overrides, removes superseded/suppressed/expired/deleted source rules, resolves conflicts by project-over-global precedence then newest authoritative source, and writes the project artifact only from the worker; Verify: compiler eligibility, lifecycle enqueue/removal, conflict, worker-only artifact write, and override-merge tests. Implemented across the T3 PR and corrective follow-up: `v065_preference_reinforcement` wires canonical machine-checkable, risk, and source-evidence state; each evidence event set counts once, disjoint evidence and overrides merge only across the same safe predicate, opposing direct saves or cleanup rewrites clear stale confidence and candidate provenance, and same-topic direct saves cannot overwrite a preference with another memory type; eligibility enforces persisted low-risk, source-trust, and review fields; the closed classifier handles directed npm/yarn/bun/pnpm choices and safe lists of forbidden commit trailers while rejecting multi-clause or reversal text; lifecycle mutations including cleanup enqueue non-lossy worker successors and periodic sweeps guarantee convergence; artifact messages and conflict diagnostics remain stable; and v065 column/index drift is guarded. Focused tests cover P1, P4, P5, precedence, evidence deduplication, direct-save/cleanup state reconciliation, cross-type topic isolation, enqueue/config failure propagation, worker-only writes, override continuity, and diagnostic recovery.
- [ ] `SP671-T4` Owner: agent; Dependencies: `SP671-T1` `SP671-T3`; Done when: `remem rules list`, `disable`, `enable`, and `set-action warn|block` expose provenance and persist overrides through artifact deletion and recompile; Verify: CLI round-trip tests.
- [ ] `SP671-T5` Owner: agent; Dependencies: `SP671-T2` `SP671-T3`; Done when: Claude Code install/dispatch supports PreToolUse Bash evaluation before command execution, PostToolUse remains capture-only, and Codex block-mode enforcement is rejected as unsupported; Verify: simulated hook integration tests for warn, block, capture-only, and unsupported-host paths.
- [x] `SP671-T6` Owner: agent; Dependencies: `SP671-T1` `SP671-T3` `SP671-T5`; Done when: `remem doctor` reports rule count, artifact presence, last compile time, last compile/evaluation error, and per-host enforcement capability without printing rule payload secrets; Verify: doctor human and JSON tests. Implemented by #840 with current artifact/compile health, payload-free project/global evaluation history, explicit Claude/Codex capability reporting, corruption visibility, and focused doctor human/JSON, hook, concurrency, privacy, and compatibility tests.
- [x] `SP671-T7` Owner: agent; Dependencies: `SP671-T3` `SP671-T5`; Done when: repeated-correction fixtures cover package-manager choice, forbidden commit trailers, and forbidden commands, and the hook latency benchmark shows p95 unchanged within measurement noise; Verify: fixture/eval commands and latency benchmark output. Implemented with a data-driven three-scenario reinforcement-to-compiler-to-hook suite and a release-mode CLI subprocess benchmark. Artifact v2 uses closed ASCII-delimited patterns with `regex-lite`, while v1 retains its original Unicode `regex` semantics. The measured enabled-rule p95 was 7.794 ms versus a 7.532 ms disabled baseline (0.262 ms delta, within the observed 0.443 ms median-absolute-deviation noise). Ordinary worktrees avoid per-hook Git subprocesses, while explicit Git layouts and discovery-control environments keep Git resolver precedence.
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
and verified. Human spec approval and the `ready_to_implement` label remain
prerequisites for any additional implementation phase.
