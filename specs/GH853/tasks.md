# Task Plan

## Linked Issue

GH-853

## Spec Packet

- Product: [`product.md`](product.md)
- Tech: [`tech.md`](tech.md)

## Implementation Tasks

- [ ] `SP853-T1` Owner: coordinator; Dependencies: none; Done when: bounded trusted traversal and tests are complete; Verify: focused graph tests
  - Owner: coordinator
  - Dependencies: none
  - Files: `src/retrieval.rs`, `src/retrieval/graph.rs`, `src/retrieval/graph/**`
  - Done when:
    - trusted one/two-edge paths, validity, caps, deterministic ordering, dedupe, and eligibility filters are implemented;
    - empty outcomes are distinct and execution errors propagate;
    - no schema, network, LLM, PPR, or context change is introduced.
  - Verify:
    - `cargo test -q retrieval::graph --lib`

- [ ] `SP853-T2` Owner: coordinator; Dependencies: `SP853-T1`; Done when: provenance-valid literal eval arm passes focused tests; Verify: focused graph decision and golden tests
  - Owner: coordinator
  - Dependencies: `SP853-T1`
  - Files: `src/eval/graph_decision.rs`, `src/eval/golden/run.rs`
  - Done when:
    - associative `hop_path` fixtures seed trusted graph edges through the typed provenance contract;
    - the graph decision report compares graph-disabled standard search with literal traversal on the same dataset/code;
    - associative primary, non-associative regression, real two-edge, leakage, and latency gates are explicit.
  - Verify:
    - `cargo test -q eval::graph_decision --lib`
    - `cargo test -q eval::golden --lib`

- [ ] `SP853-T3` Owner: coordinator; Dependencies: `SP853-T2`; Done when: a passing literal gate enables the production graph RRF channel; Verify: focused memory search tests
  - Owner: coordinator
  - Dependencies: `SP853-T2` and a passing literal gate
  - Files: `src/retrieval/search/memory/text.rs`, `src/retrieval/search/memory/weights.rs`, `src/retrieval/search/memory/tests.rs`
  - Done when:
    - only suppression-filtered FTS/vector hits seed `graph_traversal`;
    - search explain shows timing, candidates, and stable disabled reasons;
    - default graph weight is non-zero only with passing same-head evidence.
  - Verify:
    - `cargo test -q retrieval::search::memory::tests --lib`
    - `cargo run -- eval-graph-decision --json-out eval/graph-decision/report.json --json`

- [ ] `SP853-T4` Owner: coordinator; Dependencies: `SP853-T3`; Done when: report, ADR, docs, and version surfaces match the shipped behavior; Verify: deterministic and version gates
  - Owner: coordinator
  - Dependencies: `SP853-T3`
  - Files: `eval/graph-decision/report.json`, `docs/adr/2026-07-09-graph-gate-associative-followup.md`, `docs/graph-contract.md`, `docs/specs/README.md`, `README.md`, `docs/ARCHITECTURE.md`, and version-sync surfaces
  - Done when:
    - fresh same-head report satisfies every gate and the ADR records the measured result;
    - user-facing and architecture docs no longer claim that retrieval never reads `graph_edges`;
    - all required package/plugin/runtime/npm versions agree.
  - Verify:
    - `python3 scripts/ci/check_plugin_version_sync.py`
    - `cargo run -- eval-extraction --json --check-baseline`
    - `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`

- [ ] `SP853-T5` Owner: coordinator; Dependencies: `SP853-T4`; Done when: full local verification and issue-closing PR are current; Verify: full preflight and exact CI watch
  - Owner: coordinator
  - Dependencies: `SP853-T4`
  - Files: no additional files except focused GH-853 fixes found by verification
  - Done when:
    - formatting, compilation, clippy, full tests, SpecRail checks, version gates, and PR preflight pass;
    - the PR body uses `Closes #853`, maps acceptance evidence, and names the spec packet;
    - `gh pr checks <n> --repo majiayu000/remem --watch --fail-fast` exits zero for the final head.
  - Verify:
    - `cargo fmt --check`
    - `cargo check`
    - `cargo clippy --all-targets -- -D warnings`
    - `cargo test`
    - `python3 checks/check_workflow.py --repo . --spec-dir specs/GH853`
    - `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`

- [ ] `SP853-T6` Owner: independent reviewer lane; Dependencies: `SP853-T5`; Done when: final PR head has a clean independent review artifact; Verify: read-only diff/spec/gate inspection
  - Owner: independent reviewer lane
  - Dependencies: final PR head from `SP853-T5`
  - Files: none; read-only lane
  - Done when:
    - reviewer verdict is `clean` or `non_blocking` with no unresolved blocking finding;
    - the artifact records the final head SHA, spec coverage, tier attestation, and no tier dispute.
  - Verify:
    - inspect `origin/main...<head>` against `specs/GH853/`
    - validate the review artifact against `schemas/review_result.schema.json`

- [ ] `SP853-T7` Owner: coordinator; Dependencies: `SP853-T5`, `SP853-T6`; Done when: serial PR gate, merge, and closure audit are remotely confirmed; Verify: current GitHub evidence
  - Owner: coordinator
  - Dependencies: `SP853-T5`, `SP853-T6`
  - Files: local runtime checkpoint and evidence only
  - Done when:
    - GraphQL shows zero unresolved review threads and merge state is clean;
    - `checks/pr_gate.py` returns `allowed` for the final head;
    - remote truth confirms the PR merged and issue #853 closed.
  - Verify:
    - `python3 checks/github_pr_evidence.py --github-repo majiayu000/remem --pr <pr-number> --review-source independent_lane --json`
    - `python3 checks/pr_gate.py --repo . --evidence <pr-evidence.json>`
    - `gh pr view <pr-number> --repo majiayu000/remem --json merged,mergeCommit`
    - `gh issue view 853 --repo majiayu000/remem --json state,closedByPullRequestsReferences`

## Parallel Split

Implementation is serial in this worktree because traversal, evaluator, search
wiring, and committed evidence depend on one another. Planner and final reviewer
lanes are read-only. Only the coordinator runs Cargo or mutates files/GitHub.

## Verification

The coordinator runs focused checks after each implementation task, then one
full Rust suite and deterministic PR preflight on the final local head. CI is
waited with the exact single blocking command required by the user. The serial
PR gate follows the independent reviewer result and CI; merge never shares a
parallel batch with gate collection.

## Handoff Notes

- Scope is GH-853 only; PPR and SessionStart/context graph wiring are deferred.
- The stale local `spec/gh853-graph-retrieval` draft is read-only evidence and
  is not the implementation branch.
- Do not merge if the literal associative gate, non-regression gate, CI,
  reviewer lane, review-thread state, or serial PR gate is not green.
