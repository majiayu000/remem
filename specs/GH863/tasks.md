# Task Plan

## Linked Issue

GH-863

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [ ] `SP863-T1` Owner: coordinator; Done when: regression fixtures cover both GH-863 bypass classes and fail against the pre-fix verifier; Verify: see SP863-T1.
- [ ] `SP863-T2` Owner: coordinator; Done when: the AST preflight rejects both bypass classes before module execution; Verify: see SP863-T2.
- [ ] `SP863-T3` Owner: coordinator; Done when: sync and SpecRail workflow contracts accept the final tree; Verify: see SP863-T3.
- [ ] `SP863-T4` Owner: coordinator; Done when: required Rust gates pass from this worktree; Verify: see SP863-T4.
- [ ] `SP863-T5` Owner: independent reviewer lane; Done when: final-head review has no unresolved blocking findings; Verify: see SP863-T5.
- [ ] `SP863-T6` Owner: coordinator; Done when: current PR gates pass, the PR merges, and closure audit confirms GH-863 closed; Verify: see SP863-T6.

### SP863-T1 — Add regression-first bypass fixtures

- Owner: coordinator
- Dependencies: complete product and tech specs
- Files: `scripts/ci/test_specrail_gate_wiring.py`
- Covers: `B-001`, `B-002`, `B-003`, `B-004`, `B-005`
- Done when:
  - Isolated temporary-pack fixtures cover importlib loader construction and
    direct/aliased `exec`/`eval`, including module-loader metadata, frozen
    importlib modules, and indirect dynamic-namespace access.
  - Fixtures assert stable diagnostics and prove the helper sentinel never
    executes.
  - At least one GH-863 fixture fails against the pre-fix verifier.
- Verify:
  - `python3 scripts/ci/test_specrail_gate_wiring.py`

### SP863-T2 — Reject loader and code-execution surfaces

- Owner: coordinator
- Dependencies: `SP863-T1`
- Files: `scripts/sync-specrail-checks.sh`
- Covers: `B-001` through `B-008`
- Done when:
  - The AST preflight rejects non-allowlisted importlib namespaces and dynamic
    code-execution references before the classified-module import loop.
  - Existing classified static and literal dynamic import behavior remains
    green.
- Verify:
  - `python3 scripts/ci/test_specrail_gate_wiring.py`

### SP863-T3 — Validate sync and workflow contracts

- Owner: coordinator
- Dependencies: `SP863-T2`
- Files: no additional production files
- Covers: all product acceptance criteria
- Done when:
  - The sync verifier accepts the repository tree.
  - The workflow pack and GH-863 packet validators pass.
- Verify:
  - `scripts/sync-specrail-checks.sh --verify`
  - `python3 checks/check_workflow.py --repo .`
  - `python3 checks/check_workflow.py --repo . --spec-dir specs/GH863`

### SP863-T4 — Run repository-required local gates

- Owner: coordinator
- Dependencies: `SP863-T3`
- Files: no additional files
- Covers: repository submission policy
- Done when:
  - Rust formatting, compilation, and tests pass from the GH-863 worktree.
  - No Cargo command runs from another checkout.
- Verify:
  - `cargo fmt --check`
  - `cargo check`
  - `cargo test`

### SP863-T5 — Independent final-head review

- Owner: independent reviewer lane
- Dependencies: committed implementation head from `SP863-T4`
- Files: read-only review of the GH-863 diff and spec packet
- Covers: product/spec conformance, correctness, regressions, and test quality
- Done when:
  - The reviewer reports no blocking finding, or concrete findings are fixed
    and re-reviewed.
  - Reviewer evidence records the reviewed head SHA.
- Verify:
  - Native reviewer-lane result and recorded head SHA

### SP863-T6 — Gate, merge, and audit closure

- Owner: coordinator
- Dependencies: `SP863-T5`
- Files: PR and gate evidence only
- Covers: final issue closure
- Done when:
  - The one final-slice PR closes GH-863.
  - Blocking CI watch is green, review threads are resolved, PR gate and
    runtime ledger gate allow merge, remote merge is confirmed, and closure
    audit confirms the issue is closed.
- Verify:
  - `gh pr checks <n> --repo majiayu000/remem --watch --fail-fast`
  - SpecRail PR evidence and gate commands
  - Fresh GitHub issue and PR queries

## Parallelization

The writable tasks are serial: the regression fixture establishes the failing
behavior, the verifier change satisfies that fixture, and shared verification
must run against one stable head in this worktree. No two lanes may run Cargo
concurrently in this worktree.

The independent reviewer lane is read-only and starts only after the
implementation is committed. The coordinator remains the exclusive owner of
all edits, shared verification, GitHub writes, PR gating, and merge/closure
actions.

## Verification

- `python3 scripts/ci/test_specrail_gate_wiring.py`
- `scripts/sync-specrail-checks.sh --verify`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH863`
- `cargo fmt --check`
- `cargo check`
- `cargo test`
- Repository PR preflight with the intended PR body.
- Independent reviewer lane tied to the final head SHA.
- Blocking GitHub CI watch, GraphQL review-thread evidence, PR gate, runtime
  ledger gate, remote merge confirmation, and issue closure audit.

## Handoff Notes

- Queue scope is `bounded_tranche`; issue #863 is the only allowed issue.
- Authorization is `auth_mode: auto` from the current `implx auto` invocation,
  including standing merge authorization after all current gates pass.
- The PR is `pr_tier: standard`, `pr_kind: mixed_impl`, and
  `completion_mode: final`; use a closing reference for GH-863.
- Duplicate-work evidence collected before planning found no open PR or remote
  branch for GH-863.
- The threat model remains accidental or honest-maintainer bypasses. A
  malicious committer who can edit the verifier is explicitly out of scope.
- All Cargo commands must run only inside the GH-863 worktree and must be
  serialized.
