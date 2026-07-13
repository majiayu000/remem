# Task Plan

## Linked Issue

GH-813

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## 实现任务

- [ ] `SP813-T1` Owner: agent; Dependencies: spec PR approval; Files: `docs/specs/preference-rule-compilation/PRODUCT.md`, `docs/specs/preference-rule-compilation/TECH.md`, `specs/GH671/product.md`, `specs/GH671/tech.md`; Done when: GH-671 authoritative contract 与 SpecRail packet 枚举完整 eligibility allowlist、两个独立 risk 来源和 unknown-value fail-closed 要求；Verify: GH671/GH813 workflow spec checks、`git diff --check`。本任务随 #813 spec PR 获批合并后完成。
- [ ] `SP813-T2` Owner: agent; Dependencies: `SP813-T1` `SP813-T5`; Files: `src/rules/compiler.rs`, `src/rules/compiler/tests.rs`; Done when: machine-readable sensitive gate 已在 remem 对本任务生效后，compiler 以 typed policy 集中执行 fail-closed eligibility，修正现存 global-owner 过宽问题，project owner 严格按 `target_project` → `owner_key` → legacy `project` 解析且高优先级错配不能被低优先级掩盖，完整正例、逐维反例、两个独立 risk、closed-enum coverage、unknown values 和关键交叉状态均由行为测试覆盖，且不使用 SQL 字符串 snapshot；Verify: route/pr gate 先将该 diff 分类为 `enforcement_sensitive=true` 并验证 approved spec，再运行 `cargo test rules::compiler -- --nocapture`, `cargo fmt --check`, `cargo check`。
- [ ] `SP813-T3` Owner: upstream SpecRail contributor; Dependencies: `SP813-T1`; Files: upstream workflow/checkpoint/PR-evidence schema、sensitive registry input、review artifact schema、review JSON gate、GitHub evidence/thread rollup、pre-merge PR gate、executable closure audit/merge-wrapper 及其 tests; Done when: machine-readable `enforcement_sensitive` 分类被 route/pr gate 强制，独立审查证据要求 terminal state、reviewer lane、exact head、开始/完成时间和非空结果；新 head artifact 完整 carry forward 上一 head findings 的稳定 ID、resolved/unresolved/obsolete 状态，以及 resolved/obsolete 的关闭证据，unresolved 继续阻断；resolved actionable thread 验证 resolver identity/role，只有原 reviewer、带可验证 re-review evidence 的 successor reviewer lane 或获授权 human maintainer 能清除阻塞，implementer/orchestrator/coordinator/unknown 不能清除阻塞；pre-merge gate 不要求未来 dispatch evidence，dispatch 后的 wrapper/audit 验证 gate-before-dispatch 同-head 顺序并为外部违规 merge 输出 machine-readable violation/required follow-up；self-review 只有在存在已验证 reviewer-lane failure、同一 PR/head 的独立人类授权且后续 human final review 仍必需时才有效，其余失败/取消/过期/时序状态全部 fail closed；Verify: `python3 -m pytest tests/test_pr_gate.py tests/test_github_pr_evidence.py tests/test_review_json_gate.py tests/test_runtime_ledger_gate.py tests/test_closure_audit.py`，负例覆盖缺失/冲突 sensitive 标记、sensitive 无批准 spec、无 lane failure、无同 head 人类授权、缺 human final review、stale head、prior finding 缺失/仍 unresolved/无关闭证据、implementer/orchestrator/coordinator/unknown resolver、successor reviewer 缺 re-review evidence、dispatch-before-gate 和外部 merge 缺证据链，并有完整 finding carry-forward、合法 resolver 与 pre-merge 无 dispatch evidence 的正例。该任务是外部仓库依赖，未经明确授权不得从 remem lane 直接修改或发布。
- [ ] `SP813-T4` Owner: agent; Dependencies: `SP813-T3` upstream release; Files: `checks/specrail-sync.lock.json`、由 `scripts/sync-specrail-checks.sh` 管理的 synced files、remem sync tests; Done when: remem 固定并同步已发布的上游门禁，未对 vendored checks 留下手工漂移；Verify: `scripts/sync-specrail-checks.sh --verify`、相关 Python tests、`python3 checks/check_workflow.py --repo .`。
- [ ] `SP813-T5` Owner: agent; Dependencies: `SP813-T4`; Files: `workflow.yaml`, `.github/workflows/ci.yml`, `.github/pull_request_template.md`, `CONTRIBUTING.md`, repo-local workflow integration tests; Done when: remem 注册 machine-readable sensitive spec/path，CI 调用同步后的 gate/audit，在任何 compiler 实现开始前让 `SP813-T2` 路径被识别为 `enforcement_sensitive=true` 且要求 approved spec，`enforcement_sensitive` 无 fast path、exact-head review/prior-findings/thread-resolver 顺序、advisory 与 server-side protection 边界已记录，违规外部 merge 产生 required-follow-up evidence；Verify: workflow/schema/gate integration tests证明 compiler diff 在 gate 未启用或缺 approved spec 时被阻断、closure-audit negative fixtures、`python3 checks/check_workflow.py --repo .`、`git diff --check`。这些是高上下文文件，必须单独复核，不能由生成器静默修改。
- [ ] `SP813-T6` Owner: human repository administrator; Dependencies: spec approval and protection policy decision; Files: GitHub repository settings only; Done when: 明确记录是否需要不可绕过保护；若需要，启用 required check/review/ruleset 并留下 live API 与拒绝不合规 merge 的证据，agent 不执行权限变更、批准或合并；Verify: live GitHub branch-protection/ruleset API 与一次缺少 required check 时被拒绝的 merge 证据，或明确记录保持 advisory-only 的管理员决定。
- [ ] `SP813-T7` Owner: agent; Dependencies: `SP813-T1` `SP813-T2` `SP813-T3` `SP813-T4` `SP813-T5` and recorded `SP813-T6` decision; Files: GH-813 closure evidence only; Done when: Product acceptance criteria 全部有 exact-head 证据，CI、review、thread、protection capability 和 closure audit 结果被记录，#813 才可关闭；Verify: `cargo fmt --check`, `cargo check`, focused tests, `cargo test`, workflow checks, SpecRail sync verify 和 live PR evidence gate。

## 并行拆分

- `SP813-T3` 位于上游 SpecRail；`SP813-T4` 必须等待其发布，`SP813-T5` 再完成 remem
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
永久手改。2026-07-13 查询显示 GitHub
`main` 无 branch protection/ruleset，因此在 `SP813-T6` 由人类管理员作出并执行决定前，
仓库内 gate 只能宣称 advisory detection。spec PR 使用 `Refs #813`；只有 `SP813-T7`
满足后才可使用 `Closes #813`。
