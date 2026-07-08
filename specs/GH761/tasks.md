# GH761 Task Plan

## Linked Issue

GH-761

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [x] `SP761-T1` Owner: Implementation; Dependencies: GH-761 ready_to_implement; Done when: hook expectation, shell/exec parsing, matcher/timeout validation, stale duplicate detection, and parser-based removal live in one shared evaluator used by doctor, repair, and runtime self-check; Verify: `cargo test hook_integrity -- --nocapture`.
- [x] `SP761-T2` Owner: Implementation; Dependencies: T1; Done when: Claude `SessionStart` context emits a visible warning for incomplete/stale hooks, including suppressed gate output and DB-open error paths, without changing Codex JSON hook output; Verify: `cargo test context:: -- --nocapture`.
- [x] `SP761-T3` Owner: Implementation; Dependencies: T1; Done when: `remem install --target claude --repair` repairs only user-level Claude hooks, preserves third-party hook entries and MCP settings, handles hostless/exec-form stale remem hooks, and is idempotent; Verify: `cargo test install:: -- --nocapture`.
- [x] `SP761-T4` Owner: Implementation; Dependencies: T1-T3; Done when: doctor uses the shared evaluator, README documents repair semantics, and CI/preflight evidence is attached before merge; Verify: `cargo fmt --check`, `cargo check`, `cargo test`, and `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body-gh761-impl.md`.

## Verification

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH761`
- `cargo test hook_integrity -- --nocapture`
- `cargo test install:: -- --nocapture`
- `cargo test install::tests::repair_ -- --nocapture`
- `cargo test context:: -- --nocapture`
- `cargo test context::tests::gate_pipeline::claude_hook_warning_survives -- --nocapture`
- `cargo fmt --check`
- `cargo check`
- `cargo test`

## Handoff Notes

- This implementation may close GH-761 only after code, docs, tests, CI, reviewThreads, and `pr_gate` pass.
- Do not weaken doctor drift checks for stale MCP; hook-only repair must remain narrower than a full install.
