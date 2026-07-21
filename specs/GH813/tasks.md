# Task Plan

## Linked Issue

GH-813

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## 实现任务

- [x] `SP813-T1` Owner: agent; Dependencies: spec PR approval; Files: `docs/specs/preference-rule-compilation/PRODUCT.md`, `docs/specs/preference-rule-compilation/TECH.md`, `specs/GH671/product.md`, `specs/GH671/tech.md`; Done when: GH-671 authoritative contract 与 SpecRail packet 枚举完整 eligibility allowlist、两个独立 risk 来源和 unknown-value fail-closed 要求；Verify: GH671/GH813 workflow spec checks、`git diff --check`。本任务由 spec PR #814 exact head `c1e79e7e0cb880e2081d31b5c0bae63b4ee30cc8` 完成，merged as `e4867a0517df0d0e8487ac20d702d7a5444c321a`，CI `check` SUCCESS。
- [ ] `SP813-T2` Owner: agent; Dependencies: `SP813-T1` `SP813-T5`; Files: `src/rules/compiler.rs`, `src/rules/compiler/tests.rs`; Done when: machine-readable sensitive gate 已在 remem 对本任务生效后，compiler 以 typed policy 集中执行 fail-closed eligibility，修正现存 global-owner 过宽问题，project owner 严格按 `target_project` → `owner_key` → legacy `project` 解析且高优先级错配不能被低优先级掩盖，完整正例、逐维反例、两个独立 risk、closed-enum coverage、unknown values 和关键交叉状态均由行为测试覆盖，且不使用 SQL 字符串 snapshot；Verify: route/pr gate 先将该 diff 分类为 `enforcement_sensitive=true` 并验证 approved spec，再运行 `cargo test rules::compiler -- --nocapture`, `cargo fmt --check`, `cargo check`。 代码行为已由 PR #906 修复并通过 2,833 个 lib tests，但该 PR 在 `SP813-T5` 生效前合并，故不能追溯性声称满足本任务的流程前置条件；待 T5 完成后用当前实现做一次敏感门禁回放再勾选。
- [x] `SP813-T3` Owner: upstream SpecRail contributor; Dependencies: `SP813-T1`; Files: upstream workflow/checkpoint/PR-evidence schema、sensitive registry input、review artifact schema、review JSON gate、GitHub evidence/thread rollup、pre-merge PR gate、executable closure audit/merge-wrapper 及其 tests; Done when: machine-readable `enforcement_sensitive` 分类被 route/pr gate 强制；独立审查证据要求 terminal state、reviewer lane、exact head、开始/完成时间、非空结果、显式 clean/non-blocking verdict，并证明 current-head artifact 没有 blocking/actionable finding，`changes_requested`、其他 blocking verdict 及没有对应 GitHub thread 的 artifact-only actionable finding 均阻止 merge-ready；新 head artifact 完整 carry forward 上一 head findings 的稳定 ID、resolved/unresolved/obsolete 状态，以及 resolved/obsolete 的关闭证据，unresolved 继续阻断；resolved actionable thread 验证 resolver identity/role，只有原 reviewer、带可验证 re-review evidence 的 successor reviewer lane 或获授权 human maintainer 能清除阻塞，implementer/orchestrator/coordinator/unknown 不能清除阻塞；pre-merge gate 不要求未来 dispatch evidence，dispatch 后的 wrapper/audit 验证 gate-before-dispatch 同-head 顺序并为外部违规 merge 输出 schema-valid machine-readable violation/`required_follow_up` payload；上游只拥有检测与 payload contract，不直接创建 remem GitHub issue；self-review 只有在存在已验证 reviewer-lane failure、同一 PR/head 的独立人类授权且后续 human final review 仍必需时才有效，其余失败/取消/过期/时序状态全部 fail closed；Verify: `python3 -m pytest tests/test_pr_gate.py tests/test_github_pr_evidence.py tests/test_review_json_gate.py tests/test_runtime_ledger_gate.py tests/test_closure_audit.py`，负例覆盖缺失/冲突 sensitive 标记、sensitive 无批准 spec、`changes_requested`、其他 blocking verdict、current-head artifact-only actionable finding、无 lane failure、无同 head 人类授权、缺 human final review、stale head、prior finding 缺失/仍 unresolved/无关闭证据、implementer/orchestrator/coordinator/unknown resolver、successor reviewer 缺 re-review evidence、dispatch-before-gate 和外部 merge 缺证据链，并有 clean/non-blocking + zero-blocking-findings、完整 finding carry-forward、合法 resolver 与 pre-merge 无 dispatch evidence 的正例。上游 GH97 已由 PR #103/#115/#116 完成；维护者于 2026-07-21 授权 remem 固定包含这些能力的 exact SHA `0f903abe1794899071a9f19a4c46af1ce81129d3`。
- [x] `SP813-T4` Owner: agent; Dependencies: `SP813-T3` upstream release or maintainer-authorized exact SHA; Files: `checks/specrail-sync.lock.json`、由 `scripts/sync-specrail-checks.sh` 管理的 synced files、remem sync tests; Done when: remem 固定并同步已发布的上游门禁或维护者授权的 exact SHA，未对 vendored checks 留下手工漂移；Verify: `scripts/sync-specrail-checks.sh --verify`、相关 Python tests、`python3 checks/check_workflow.py --repo .`。 2026-07-21 已由 clean upstream checkout 同步 `checks/closure_audit.py` 与 `schemas/closure_audit_result.schema.json`，lock 保持 exact SHA `0f903abe1794899071a9f19a4c46af1ce81129d3`；sync/import/workflow wiring 测试通过。
- [x] `SP813-T5` Owner: remem workflow integration agent; Dependencies: `SP813-T4`; Files: `workflow.yaml`, `.github/workflows/ci.yml`, `.github/pull_request_template.md`, `CONTRIBUTING.md`, repo-local workflow controller/adapter and integration tests; Done when: remem 注册 machine-readable sensitive spec/path，CI 调用同步后的 gate/audit，在任何 compiler 实现开始前让 `SP813-T2` 路径被识别为 `enforcement_sensitive=true` 且要求 approved spec，`enforcement_sensitive` 无 fast path、exact-head review/prior-findings/thread-resolver 顺序、advisory 与 server-side protection 边界已记录；由有 GitHub 写权限的 remem controller 消费上游 `required_follow_up` payload，通过 GitHub Issues API 按 `repository + pr_number + final_head_sha + violation_code` 稳定幂等键创建、重新打开或复用 durable issue，并回读 issue number、URL、open state 与幂等键写入 closure evidence。只有 artifact、写入失败或回读失败均保持 closure blocked，不得声称 follow-up 已创建；Verify: workflow/schema/gate integration tests 证明 compiler diff 在 gate 未启用或缺 approved spec 时被阻断，closure-audit/controller fixtures 覆盖首次创建、重复运行复用同一 issue、已关闭 issue 重新打开、artifact-only、API 写入/回读失败，且持久化正例能由 fake GitHub adapter 回读匹配的 open issue；`python3 checks/check_workflow.py --repo .`、`git diff --check`。这些是高上下文文件，必须单独复核，不能由生成器静默修改。 2026-07-21 已接入 sensitive registry、PR 声明冲突 CI、可信 `pull_request_target` closure workflow、幂等 GitHub issue controller 与 fake-adapter 回读测试；仓库内能力保持 advisory，等待 T6 管理员 ruleset 实际设置。
- [ ] `SP813-T6` Owner: human repository administrator; Dependencies: spec approval and protection policy decision; Files: GitHub repository settings only; Done when: 明确记录是否需要不可绕过保护；若需要，启用 required check/review/ruleset 并留下 live API 与拒绝不合规 merge 的证据，agent 不执行权限变更、批准或合并；Verify: live GitHub branch-protection/ruleset API 与一次缺少 required check 时被拒绝的 merge 证据，或明确记录保持 advisory-only 的管理员决定。
- [ ] `SP813-T7` Owner: agent; Dependencies: `SP813-T1` `SP813-T2` `SP813-T3` `SP813-T4` `SP813-T5` and recorded `SP813-T6` decision; Files: GH-813 closure evidence only; Done when: Product acceptance criteria 全部有 exact-head 证据，CI、review、thread、protection capability 和 closure audit 结果被记录；若存在 `B-010` violation，closure evidence 还必须含由 GitHub API 回读且与稳定幂等键匹配的 open follow-up issue number/URL/state，只有本地 artifact 时 #813 不得关闭；Verify: `cargo fmt --check`, `cargo check`, focused tests, `cargo test`, workflow checks, SpecRail sync verify、live PR evidence gate，以及对 follow-up issue 的 live GitHub API read-back。

## 并行拆分

- `SP813-T3` 位于上游 SpecRail；`SP813-T4` 必须等待其发布或维护者明确授权 exact SHA，`SP813-T5` 再完成 remem
  registry/CI 集成。只有这条敏感门已经对 compiler 路径生效后，`SP813-T2` 才能开始，
  因而 `SP813-T2` 不得与 `SP813-T3`/`T4`/`T5` 并行抢跑。
- `SP813-T6` 是独立的人类管理员 lane，可并行做政策决策，但不能由 agent 代办。
- 同一 lane 内禁止两个 agent 同时写 `src/rules/compiler.rs`、`CONTRIBUTING.md` 或 synced
  SpecRail 文件；需要并行时必须先声明不重叠的文件所有权。

## 验证

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH813`
- `python3 checks/route_gate.py --repo . --route write_spec --issue 813 --state triaged --json`
- `scripts/sync-specrail-checks.sh --verify`
- 实现阶段：`cargo fmt --check`
- 实现阶段：`cargo check`
- 实现阶段：`cargo test rules::compiler -- --nocapture`
- 提交前：`cargo test`
- 合并准备：在 final head 上收集独立 review artifact、GitHub PR evidence、thread rollup 与
  PR gate 输出；不得使用旧 head 或调用方裸 `review_source` 作为完成证明。

## Handoff Notes

本 packet 只定义 #813 的修复契约，不批准实现。当前 `main` 已包含 #797/#801 的原始
correctness fixes，但 global-owner 过滤仍过宽；`SP813-T2` 是必需修复，而不只是防漂移
重构。为避免同类流程缺口重现，必须先完成 `SP813-T3` → `SP813-T4` → `SP813-T5`
并证明 compiler 路径已受敏感门约束，才能开始 `SP813-T2`。`checks/github_pr_evidence.py`、
`checks/pr_gate.py`、review schema、closure audit 等由上游 SpecRail 同步，不能在 remem 中
永久手改。2026-07-21 live API 仍显示 GitHub `main` 无 branch protection/ruleset；维护者
已选择 required-check ruleset，但实际设置和拒绝证据仍必须由人类管理员完成。在此之前
仓库内 gate 只能宣称 advisory detection。spec PR 使用 `Refs #813`；只有 `SP813-T7`
满足后才可使用 `Closes #813`。
