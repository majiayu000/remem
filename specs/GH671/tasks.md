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
- [ ] `SP671-T3` Owner: agent; Dependencies: `SP671-T1` `SP671-T2`; Done when: the compiler selects only eligible active preferences from canonical preference reinforcement state, merges user overrides, removes superseded/suppressed/expired/deleted source rules, resolves conflicts by project-over-global precedence then newest authoritative source, and writes the project artifact only from the worker; Verify: compiler eligibility, lifecycle enqueue/removal, conflict, worker-only artifact write, and override-merge tests. The core compiler landed across the T3 PR and corrective follow-up: `v065_preference_reinforcement` wires canonical machine-checkable, risk, and source-evidence state; each evidence event set counts once, disjoint evidence and overrides merge only across the same safe predicate, opposing direct saves or cleanup rewrites clear stale confidence and candidate provenance, and same-topic direct saves cannot overwrite a preference with another memory type; eligibility enforces persisted low-risk, source-trust, and review fields; the closed classifier handles directed npm/yarn/bun/pnpm choices and safe lists of forbidden commit trailers while rejecting multi-clause or reversal text; lifecycle mutations including cleanup enqueue non-lossy worker successors and periodic sweeps guarantee convergence; artifact messages and conflict diagnostics remain stable; and v065 column/index drift is guarded. Focused tests cover P1, P4, P5, precedence, evidence deduplication, direct-save/cleanup state reconciliation, cross-type topic isolation, enqueue/config failure propagation, worker-only writes, override continuity, and diagnostic recovery. T3 remains incomplete under the accepted closed eligibility contract until #813 enforces the exact global `user` / `user:default` / no-target owner tuple and adds the exhaustive positive, independent-negative, unknown-value, and critical cross-state matrix.
- [x] `SP671-T4` Owner: agent; Dependencies: `SP671-T1` `SP671-T3`; Done when: `remem rules list`, `disable`, `enable`, `set-action <rule_id> warn [--host claude-code|codex-cli]`, and `set-action <rule_id> block --host claude-code` expose provenance and persist overrides through artifact deletion and recompile; `--host` is optional and does not gate warn, while block without the explicit Claude Code host is rejected; Verify: CLI round-trip tests. #837 at merge commit `4d5eafa9b217950b91e8cb46c20c52ce3d9de4a8` implements the management/CLI foundation (`src/cli/actions/rules.rs:10-84`) and warn disable/enable/action round trip with worker rebuild (`src/rules/management/tests.rs:82-138`). The shared unsupported-pre-execution test (`src/rules/management/tests.rs:142-174`) exercises one capability=false rejection path and proves no override or compile job is written; it does not separately execute host None and Codex CLI cases. #839 exact head `905a55f7219459dd7b33a1805f0d4da27a97622f`, merged as `f612b4a1ec4558ed6d2df85699cefb42109bdf7c`, adds supported Claude block persistence (`src/rules/management/tests.rs:177-204`). Existing compiler coverage reconstructs stored disabled/action overrides (`src/rules/compiler/tests.rs:380-399`).
- [x] `SP671-T5` Owner: agent; Dependencies: `SP671-T2` `SP671-T3`; Done when: Claude Code install/dispatch supports PreToolUse Bash evaluation before command execution, PostToolUse remains capture-only, and Codex block-mode enforcement is rejected as unsupported; Verify: simulated hook integration tests for warn, block, capture-only, and unsupported-host paths. Implemented by #839 exact head `905a55f7219459dd7b33a1805f0d4da27a97622f`, merged as `f612b4a1ec4558ed6d2df85699cefb42109bdf7c`; verified by `rules::hook::tests` for visible warn and pre-execution deny, `install::tests::build_hooks_contains_expected_*` for Claude PreToolUse/PostToolUse separation and absent Codex enforcement hooks, `observe::tests::successful_explicit_commit_persists_full_git_evidence` for PostToolUse capture, and the shared unsupported-host hook path.
- [x] `SP671-T6` Owner: agent; Dependencies: `SP671-T1` `SP671-T3` `SP671-T5`; Done when: `remem doctor` reports rule count, artifact presence, last compile time, last compile/evaluation error, and per-host enforcement capability without printing rule payload secrets; Verify: doctor human and JSON tests. Implemented by #840 at merge commit `ca1a804c8f8b8889ac8b2ba29f5f1c8522f17884` with current artifact/compile health, payload-free project/global evaluation history, explicit Claude/Codex capability reporting, corruption visibility, and focused doctor human/JSON, hook, concurrency, privacy, and compatibility tests.
- [x] `SP671-T7` Owner: agent; Dependencies: `SP671-T3` `SP671-T5`; Done when: repeated-correction fixtures cover package-manager choice, forbidden commit trailers, and forbidden commands, and the hook latency benchmark passes both fixed budgets (enabled p95 `<= 15.0 ms`; enabled-minus-disabled p95 delta `<= 1.0 ms`), with MAD retained only as informational output; Verify: fixture/eval commands and latency benchmark output. Implemented with a data-driven three-scenario reinforcement-to-compiler-to-hook suite and a release-mode CLI subprocess benchmark. Artifact v2 uses closed ASCII-delimited package-manager patterns with `regex-lite` and a structural Git force-push predicate, while v1 retains its original Unicode `regex` semantics. Structural regression coverage includes line continuations, legal group boundaries, quoted/echoed text, literal and effective-fd0 shell-stdin heredocs, inherited descriptor duplication, `source /dev/stdin`, force refspecs, deletion and option-value edges, local/default Git config, `core.worktree`, incomplete or malformed nested markers, invalid or unreadable HEAD state, gitfile conformance, linked worktrees, filesystem device boundaries, discovery-control environments, `command`/`env`/`exec` wrappers, argv-correct `env -S`, static and repeated `builtin eval`, EXIT trap query/replacement/reset/early-exit behavior, executable versus noexec shell payloads, mirror abbreviations and boolean negation, path-qualified Git executables, static `&&`/`||`, `if`/`elif`, loop, and nocasematch-aware case reachability, ANSI-C quoting, invoked/unset/scoped/exported/positional-argument function definitions with quote-aware concatenation, recursive defaults, and unquoted splitting behavior, parse-time shell aliases and Git-config aliases with Git-native ordinary-alias tokenization, job-control-aware `lastpipe`, assignment-word and parameter/arithmetic substitutions, package-command redirections, static brace alternatives, bounded order-preserving semantic critical-variant projection, and continuation after bounded expansion exhaustion. The final-head fixed-budget artifact measured baseline p95 `7.627417 ms`, enabled p95 `7.905666 ms`, delta `0.278249 ms`, complex-AST p95 `7.921000 ms`, and MAD `0.313583 ms`; it passes both fixed budgets. Plain validated `.git` directories and plain Git config avoid per-hook Git subprocesses, while gitfiles, symlinks, filesystem device boundaries, incomplete or explicit Git layouts, and worktree-affecting config keep Git resolver precedence.
- [ ] `SP671-T8` Owner: agent; Dependencies: `SP671-T1` `SP671-T2` `SP671-T3` `SP671-T4` `SP671-T5` `SP671-T6` `SP671-T7`; Done when: docs and architecture notes reflect the shipped behavior, all acceptance criteria are checked, and #671 is updated with implementation evidence; Verify: `cargo fmt --check`, `cargo check`, focused tests, `cargo test`, `python3 checks/check_workflow.py --repo . --spec-dir specs/GH671`, and `git diff --check`. T8a reconciles the shipped CLI/doctor evidence and public documentation, but T8 remains incomplete and #671 stays open until #813 completes T3's global-owner correction and exhaustive eligibility matrix.

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
prerequisites for any additional implementation phase. #860 and #861 are
separate follow-up backlog and #863 is unrelated to GH671 acceptance; none of
them counts as completing GH671 or authorizes closing it.
