# GH761 Task Plan

## Linked Issue

GH-761

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## 实现任务

- [ ] `SP761-T1` Owner: hook-integrity implementer; Dependencies: none; Done when: doctor hook expectation and parsing are available through a shared evaluator that reports Claude registered/expected/missing events and current settings path; Verify: focused evaluator tests for 5/5, 3/5, missing file, invalid JSON, and current binary/host matching.
- [ ] `SP761-T2` Owner: context/runtime implementer; Dependencies: `SP761-T1`; Done when: Claude SessionStart context output includes a visible hook integrity warning when Claude hooks are incomplete or unreadable, while complete hooks and Codex JSON output remain unchanged; Verify: context rendering tests for stale Claude hooks, healthy Claude hooks, and Codex wrapper compatibility.
- [ ] `SP761-T3` Owner: install implementer; Dependencies: `SP761-T1`; Done when: `remem install --target claude --repair` restores the expected five Claude hook entries without touching MCP/runtime store/API token and preserves third-party hook entries; Verify: repair, dry-run, idempotency, third-party preservation, and invalid JSON failure tests.
- [ ] `SP761-T4` Owner: doctor/docs implementer; Dependencies: `SP761-T1`, `SP761-T2`, `SP761-T3`; Done when: doctor points partial Claude hook setups to the repair command, README/CLI help explain repair and SessionStart warnings, and changelog/version-sync files are updated if implementation changes shipped binaries; Verify: doctor tests, docs review, version sync check, and preflight.

## 并行拆分

- `SP761-T1` must land before runtime warning or repair implementation to keep one source of hook truth.
- `SP761-T2` and `SP761-T3` can be developed in parallel after `SP761-T1` if file ownership is disjoint: context files for T2, install/CLI files for T3.
- `SP761-T4` should wait until warning wording and CLI behavior are final.

## 验证

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH761`
- Focused Rust tests named by each implementation PR.
- `cargo fmt --check`
- `cargo check`
- `cargo test` before final closure.

## Handoff Notes

- Spec-only PR uses `Refs #761`.
- Runtime implementation PRs use `Refs #761` until all tasks above are complete.
- Final implementation PR may use `Closes #761` only after runtime warning, repair, doctor compatibility, docs, tests, and full verification have landed.
- Do not close GH-761 from the spec-only PR.
