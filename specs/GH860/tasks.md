# Task Plan

## Linked Issue

GH-860

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative contract:
  `docs/specs/preference-rule-compilation/TECH.md`

## Implementation Tasks

- [x] `SP860-T1` Owner: coordinator; Dependencies: complete spec packet and duplicate-work route gate; Done when: paired red fixtures demonstrate the current failures for supported `.exe` shell basenames and `bash -c` positional binding while nearby unrelated or missing-argument forms remain allowed; Verify: named red logs for `force_push_rule_recognizes_exe_shell_basenames` and `force_push_rule_binds_shell_command_positional_parameters`. Covers: B-001, B-002, B-003, B-004, B-008.
- [x] `SP860-T2` Owner: coordinator; Dependencies: `SP860-T1`; Done when: supported exact `.exe` basenames normalize to their suffix-free shell name and a static shell `-c` payload receives the correct `$0` and positional-argument mapping, including field cardinality, default/alternative words, slices/substrings, separately evaluated bounded `set --` variants in whole or concatenated words, `set -`, `shift`, heredoc ownership, sourced-file arguments, function-shadowed positional command names, isolated full shell-state alternatives, fallible setup, normalized wrapper status, and terminated-path EXIT traps; Verify: `cargo test -q rules::evaluator --lib`. Covers: B-001, B-002, B-003, B-004, B-006, B-007, B-008.
- [x] `SP860-T3` Owner: coordinator; Dependencies: complete spec packet and duplicate-work route gate; Done when: paired red fixtures demonstrate that plain function-shadowed `unset -f` currently mutates the wrong function state while explicit `builtin unset -f` retains builtin semantics; Verify: named red log for `force_push_rule_resolves_unset_function_before_builtin_state`. Covers: B-005, B-006, B-008.
- [x] `SP860-T4` Owner: coordinator; Dependencies: `SP860-T3`; Done when: static function resolution precedes plain builtin-like `unset` mutation while explicit builtin-selection forms still update function state; Verify: `cargo test --lib rules::evaluator::tests::git_execution`. Covers: B-005, B-006, B-007, B-008.
- [x] `SP860-T5` Owner: coordinator; Dependencies: `SP860-T2`, `SP860-T4`; Done when: the authoritative technical contract names the shipped semantics and all required release surfaces stage `0.6.9` coherently; Verify: `python3 scripts/ci/check_plugin_version_sync.py` and `python3 scripts/ci/check_version_bump.py origin/main HEAD`. Covers: B-007; release metadata itself covers no additional behavior invariant.
- [ ] `SP860-T6` Owner: coordinator; Dependencies: `SP860-T5`; Done when: the current diff passes focused, workflow, formatting, build, full-suite, clippy, deterministic eval, and PR preflight gates; Verify: every command in the Verification section exits zero with a retained log. Covers: B-001, B-002, B-003, B-004, B-005, B-006, B-007, B-008.
- [ ] `SP860-T7` Owner: independent reviewer lane; Dependencies: pushed PR head for `SP860-T6`; Done when: a native read-only reviewer checks the final diff against every invariant, no blocking finding remains, CI is green, review threads are resolved, and the serial PR gate allows merge; Verify: reviewer artifact, `gh pr checks <n> --repo majiayu000/remem --watch --fail-fast`, GraphQL review-thread evidence, and `pr_gate.py`. Covers: B-001, B-002, B-003, B-004, B-005, B-006, B-007, B-008.

## Parallelization

Implementation is serial in this worktree because the three changes converge
in `CommandCollector::collect_static_tokens` and the same regression tables.
Cargo commands are coordinator-only and never concurrent. After the PR head is
pushed and CI evidence is available, one native reviewer lane is read-only and
owns no writable files.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH860`
- `python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem --issue 860 --json`
- `python3 checks/route_gate.py --repo . --route implement --issue 860 --state ready_to_implement --duplicate-evidence <path> --json`
- `cargo test -q rules::evaluator --lib`
- `python3 scripts/ci/check_plugin_version_sync.py`
- `node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/server.test.js npm/remem/scripts/install.test.js`
- `python3 scripts/ci/check_version_bump.py origin/main HEAD`
- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `cargo run -- eval-extraction --json --check-baseline`
- `cargo run -- eval-gates --json-out artifacts/logs/gh860/remem-eval-gates.json`
- `cargo clippy --all-targets -- -D warnings`
- `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file <path>`

## Handoff Notes

- Queue scope is only GH-860; do not inspect, label, implement, or close any
  other issue as part of this tranche.
- Current live scope is the three-item remainder recorded by the owner after
  PR #841 merged: `.exe` shell basename recognition, shell `-c` positional
  binding, and function-shadowed `unset` ordering.
- This is one `standard` `mixed_impl` PR and the final slice, so its body may
  use `Closes #860` only after all eight invariants are implemented and tested.
- The current `implx auto` invocation supplies bounded spec drafting,
  readiness-label, implementation, and merge authorization, but does not
  remove independent review, CI, review-thread, PR-gate, or clean-merge-state
  requirements.
