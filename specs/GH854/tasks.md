# Task Plan

## Linked Issue

GH-854

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [x] `SP854-T1` — Extract shared relevance and add deterministic selection — Owner: coordinator; Dependencies: approved GH-854 packet; Done when: Core is frozen unchanged and k=0/tie/sparse/blank cases pass; Verify: focused context relevance and policy tests
  - Files: `src/context/relevance.rs`, `src/context/prompt_submit.rs`,
    `src/context/policy.rs`, `src/context/types.rs`, `src/context/query.rs`.

- [x] `SP854-T2` — Integrate SessionStart selection, audit, and footer — Owner: coordinator; Dependencies: `SP854-T1`; Done when: low relevance, k, section, and final truncation remain distinguishable on both hosts; Verify: focused render and audit tests
  - Files: `src/context/render.rs`, `src/context/render/stats.rs`,
    `src/context/style.rs`, `src/context/audit.rs`, context tests.

- [x] `SP854-T3` — Add latest-session relevance status — Owner: coordinator; Dependencies: `SP854-T2`; Done when: latest policy state/counts are visible and legacy rows report unavailable; Verify: focused DB status and CLI text/JSON tests
  - Files: `src/db/query/status_spend.rs`,
    `src/cli/actions/query/status.rs`, `src/cli/actions/query/status/*`.

- [x] `SP854-T4` — Produce four-arm report and apply selected default — Owner: coordinator; Dependencies: `SP854-T1`; Done when: k 1/3/5/10, every populated slice, SessionStart chars, capacity, hashes, decision, and secondary tradeoffs are committed; Verify: report completeness and decision tests plus existing eval commands
  - Files: `eval/sessionstart-k-sweep/report.json`,
    `eval/sessionstart-k-sweep/README.md`, focused eval tests as needed.

- [x] `SP854-T5` — Update docs, versions, and heavy-tier evidence — Owner: coordinator; Dependencies: `SP854-T1`–`SP854-T4`; Done when: docs match implementation and every required local/SpecRail/version gate passes; Verify: full verification list below

## Parallel Split

The runtime tasks overlap in `src/context/` and are intentionally serialized in
one worktree. Independent spec review, final PR review, and closure audit are
read-only native lanes.

## Verification

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH854`
- Focused context relevance/render/audit/status/eval tests
- `cargo fmt --check`
- `cargo check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`
- `python3 scripts/ci/check_plugin_version_sync.py`
- `python3 scripts/ci/check_version_bump.py origin/main HEAD`
- `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`

## Handoff Notes

The maintainer comment on issue #854 dated 2026-07-19 is the approved charter.
This packet deliberately removes the stale draft's coding-bench, signed-tag,
trust-root, schema, and suppression-transaction expansion. PR tier is `heavy`;
the user supplied standing merge authorization through `implx auto`, but CI,
independent review, review-thread, and PR gates remain mandatory.
