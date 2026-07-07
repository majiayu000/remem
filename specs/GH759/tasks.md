# GH759 Task Plan

## Linked Issue

GH-759

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## 实现任务

- [ ] `SP759-T1` Owner: runtime/user-context implementer; Dependencies: none; Done when: `user_context.auto_promote` config is parsed with defaults, validation, and `strict` rollback policy; Verify: focused config unit tests such as `cargo test runtime_config`.
- [ ] `SP759-T2` Owner: user-context extraction implementer; Dependencies: `SP759-T1`; Done when: extraction and candidate-apply paths share one `AutoPromotePolicy`, default policy lowers confidence only, `strict=true` preserves the old hard gate behavior, and non-default source_kind/text-support relaxation is either fully implemented with prompt/queue-support/non-retention source scanning or left disabled by default; Verify: focused `user_context::extraction` and `user_context::candidates` tests.
- [ ] `SP759-T3` Owner: safety/regression implementer; Dependencies: `SP759-T2`; Done when: non-retention, sensitivity, risk, third-party framing, user-authored source, claim-key conflict, and existing adversarial fixtures still fail closed under relaxed config; Verify: existing adversarial user-context regression suite plus new relaxed/strict comparison fixtures.
- [ ] `SP759-T4` Owner: observability/docs implementer; Dependencies: `SP759-T1`, `SP759-T2`, `SP759-T3`; Done when: README/config-facing docs, release notes, and `docs/specs/user-context-layer/TECH.md` describe the new default, strict rollback, unchanged hard gates, governance path, and any user-context candidate/claim stats used for verification; Verify: `python3 scripts/ci/check_pr_preflight.py --fast --base origin/main --pr-body-file <body>`.

## 并行拆分

- `SP759-T1` touches runtime/config surfaces and should land before writable implementation lanes.
- `SP759-T2` and `SP759-T3` overlap in `src/user_context/**`; keep them serial unless the first PR exposes a stable policy API and the second PR is test-only.
- `SP759-T4` can be prepared read-only in parallel, but documentation edits should wait until the final implementation behavior is fixed.

## 验证

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH759`
- Focused Rust tests named by each implementation PR.
- `cargo fmt --check`
- `cargo check`
- `cargo test` before final closure.

## Handoff Notes

- Spec-only PRs use `Refs #759`; implementation PRs use `Refs #759` until every task above is complete.
- Do not close GH-759 until relaxed default behavior, strict rollback, unchanged safety boundaries, docs, and full verification have landed.
- GH-760 handles historical preference backfill and remains a separate issue even though the two features complement each other.
