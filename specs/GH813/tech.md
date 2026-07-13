# Tech Spec

## Linked Issue

GH-813

## Product Spec

Product: `product.md`

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Preference eligibility | `src/rules/compiler.rs`, `src/rules/compiler/tests.rs` | SQL 已检查 lifecycle、expiry、scope、source trust、machine-checkable、reinforcement threshold/risk、candidate risk/review status 和 suppression policy；但 global scope 只要求 `owner_scope IS NOT NULL`，且测试 fixture 让两个 risk 共用字段。 | 需要修正现存 global-owner 过宽问题，把 eligibility 变成封闭、可枚举、fail-closed 的契约，并证明每个条件不可遗漏。 |
| GH-671 contracts | `specs/GH671/product.md`, `specs/GH671/tech.md`, `docs/specs/preference-rule-compilation/PRODUCT.md`, `docs/specs/preference-rule-compilation/TECH.md` | 现有契约概括了 low-risk、trusted source、scope 和 machine-checkable，但没有枚举全部 review/trust/risk 允许值与交叉状态。 | `B-001` 至 `B-005` 要求 authoritative contract 与第一轮实现测试一致。 |
| Review artifact | `schemas/review_result.schema.json`, `checks/review_json_gate.py` | artifact 可带 `head_sha`、`review_round` 和 `mode`，但 schema/gate 不要求独立 reviewer 身份、开始/完成时间或终态。 | 仅有 PASS 文本不足以证明独立审查在合并前完成。 |
| PR evidence and gate | `checks/github_pr_evidence.py`, `checks/pr_gate.py` | evidence 接受调用方传入的 `review_source`；gate 检查 exact head、threads 与 merge ordering，但没有加载并验证对应的独立审查 artifact。 | 必须从可验证产物建立 review completion → gate → merge 的时序链。 |
| Runtime ledger | `schemas/runtime_checkpoint.schema.json`, `checks/runtime_ledger_gate.py` | queue ledger 已记录 reviewer lane 和 review source，并可阻止 agent 自主推进。 | 可复用 lane/失败状态语义，但不能把本地 ledger 误称为 GitHub 服务端保护。 |
| Synced SpecRail checks | `scripts/sync-specrail-checks.sh`, `checks/specrail-sync.lock.json` | review schema、GitHub evidence 和 PR gate 从上游 `majiayu000/specrail` 固定版本同步。 | remem 不得直接修改这些 vendored 文件；需要上游变更后再同步。 |
| Repository governance | `workflow.yaml`, `CONTRIBUTING.md` | workflow 禁止 agent 最终批准、合并和权限变更；CONTRIBUTING 尚未说明 enforcement-sensitive 无 fast path。GitHub `main` 在 2026-07-13 无 branch protection/ruleset。 | 仓库内门禁只能约束 agent 流程；不可绕过的 merge enforcement 需要人类管理员配置服务端规则。 |

## 设计方案

### 1. remem 本地 eligibility contract

- 在 GH-671 authoritative Product/Tech spec 中列出封闭资格表：`preference`、active/
  unexpired、project+repo+current target 或 global+user+`user:default`+no target、三种
  trusted source、machine-checkable、threshold、两个独立 low-risk、三个 reviewed status、
  policy evaluation success 且无匹配 active suppression。其他值全部 ineligible。
- 在 `src/rules/compiler.rs` 中引入 typed `RuleEligibilityInput`、封闭的
  `RuleEligibilityDecision`/`RejectReason`（或等价结构）和纯函数
  `eligibility_decision()`，避免 allowlist 分散在长 SQL 与不相关代码路径。SQL 只负责
  参数化 join、project/global 候选范围和读取判定所需字段；安全资格由纯策略统一决定，
  未知数据库值必须产生明确 deny/error diagnostic，不能静默放行。
- project owner 使用单一解析优先级：`target_project` 的非空值优先，其次 `owner_key`，
  最后才是 legacy `project`；高优先级值指向其他 repo 时不得被低优先级匹配掩盖。
  global owner 必须精确为 `user` / `user:default` / no target；这是一项现存 correctness
  修复，不只是结构化重构。
- `src/rules/compiler/tests.rs` 使用行为矩阵：从一个完整 eligible fixture 出发，每次只改变
  一个维度并断言 ineligible；fixture 将 candidate risk 与 reinforcement risk 拆成独立
  字段，再加入高风险与已批准、低风险与未审查等关键交叉反例。
- 对 memory/review/trust/risk 等封闭类型提供 `ALL` 或等价的全量 variant 枚举，测试比较
  policy 的 allow/deny coverage union。新增 variant 必须暴露未分类项。对数据库未知字符
  串断言 ineligible。测试固定结果集合和 provenance，不对 SQL 空白、clause 顺序或完整
  字符串做 snapshot。

### 2. 上游 SpecRail review completion evidence

以下文件由 `checks/specrail-sync.lock.json` 管理，必须先在上游 SpecRail 实现和发布，
remem 只通过 `scripts/sync-specrail-checks.sh` 同步：

- 扩展 review artifact，至少要求 `reviewer_lane`（或等价独立身份）、`head_sha`、
  `review_started_at`、`review_completed_at`、终态和 verdict。取消、失败、空输出与
  superseded 必须是可区分状态。
- review artifact 对每个 superseded head 携带 `prior_findings`：稳定 finding ID、来源
  head、`resolved|unresolved|obsolete` 状态及 resolved/obsolete 证据。gate 读取上一有效
  artifact 并验证新 artifact 完整覆盖旧 findings；缺项、重复/冲突状态、无关闭证据或
  unresolved 均 fail closed。
- `review_json_gate` 校验终态、时间顺序、非空结果和 exact head，而不只校验 verdict 文本。
- `github_pr_evidence` 从经 schema/gate 验证的 artifact 生成 `review_source` 和完成证据；
  merge-ready 路径不再接受裸字符串作为独立审查证明。
- workflow/checkpoint/PR-evidence schema 增加 machine-readable
  `enforcement_sensitive` 分类；remem 的敏感 spec/path registry 把 preference compiler 等
  自动强制路径映射为 true。route/pr gate 拒绝缺失、与 registry 冲突或没有已批准 spec 的
  sensitive evidence，并由负例测试固定。
- pre-merge `pr_gate` 只要求
  `review_completed_at <= gate_started_at <= gate_completed_at`，并绑定 review、CI、thread
  rollup 与 PR current head；它不要求未来的 merge dispatch 证据。
- 上游提供可执行 closure-audit/merge-wrapper 检查（随 SpecRail lock 同步），在 dispatch
  evidence 存在后验证 `gate_completed_at < merge_dispatched_at <= merged_at` 和同一 final
  head；外部 merge 缺少该链时输出 machine-readable violation 和 required follow-up。
- self-review evidence 只有在存在可验证的 reviewer-lane failure、同一 PR/head 的独立
  人类授权且后续仍有 human final review 时才有效；否则 fail closed。
- thread rollup 对 resolved actionable reviewer/human thread 保留 resolver identity、
  verified role 和授权来源；只有原 reviewer、带可验证 re-review evidence 的 successor
  reviewer lane 或获授权 human maintainer 可清除阻塞。resolver 缺失、unknown、
  implementer、orchestrator 或 coordinator 的 resolved thread 仍 fail closed。
- 增加 fixture：pending、failed、cancelled、empty、stale head、review-after-merge、
  prior finding 缺失/无状态/状态仍为 unresolved/无 resolved-or-obsolete 证据、
  unresolved thread、由 implementer/orchestrator/coordinator/unknown resolver 解决的
  actionable thread、无 re-review evidence 的 successor reviewer resolver、artifact
  unreadable、伪造 source、无授权 self-review、缺失/冲突 enforcement classification、
  pre-merge 缺失未来 dispatch 的正例、dispatch-before-gate，以及外部 merge 无证据链。

### 3. remem 同步与仓库流程说明

- 上游变更发布后更新 `checks/specrail-sync.lock.json` 并通过同步脚本更新 vendored 文件；
  禁止在 remem 内对 synced 文件做永久性手改。
- 在 `CONTRIBUTING.md` 增加 `enforcement_sensitive` 分类、无 fast path、exact-head review
  顺序和 agent 权限边界。避免修改 `AGENTS.md`，除非维护者另行要求高上下文规则变更。
- 同步并接入上游可执行 closure audit；如果已合并 PR 缺少合规 review completion 链，
  输出 gate violation 和 required-follow-up artifact，由有 GitHub 写权限的控制器创建或
  保留 issue/任务。

### 4. GitHub 服务端保护的人类边界

- 记录 `main` 的 branch protection/ruleset 查询结果和时间。
- 若项目要求“外部或管理员也不能绕过”，由有权限的人类管理员配置 required check、
  required review 或 ruleset，并验证实际拒绝缺少 gate check 的 merge。
- agent 不修改权限、不批准、不合并。服务端保护未启用时，工具必须把能力描述为
  advisory detection，而不是 prevention。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001` | workflow/checkpoint/PR-evidence schema、sensitive registry、route/pr gates | 缺失标记、registry 冲突、sensitive 无批准 spec 的 fixtures 均失败；有效 true + approved spec 通过 |
| `B-002` | compiler eligibility policy、GH-671 specs | 完整 eligible fixture；逐维 allowlist 表与测试集合一致 |
| `B-003` | compiler query/policy | table-driven 单变量反例全部不产生 compiled rule |
| `B-004` | compiler tests | 正例、逐维反例和关键交叉反例在同一实现 PR 通过 |
| `B-005` | typed parsing/query policy | unknown trust/risk/review/scope/lifecycle fixtures 均 ineligible |
| `B-006` | upstream review schema/json gate/evidence | pending、failed、cancelled、empty、unreadable artifact 均阻止 merge-ready |
| `B-007` | upstream review artifact/evidence/pr gate | stale-head、新 head 遗漏 prior finding、carry-forward 后仍 unresolved、resolved/obsolete 无证据 fixtures 失败；完整 carry-forward + 有关闭证据 + exact-head review 通过 |
| `B-008` | upstream PR evidence/thread rollup | unresolved reviewer/human thread、implementer/orchestrator/coordinator/unknown resolver，以及 successor reviewer 缺 re-review evidence 的 fixtures 失败；原 reviewer、有 re-review evidence 的 successor reviewer lane、获授权 human maintainer resolver 通过 |
| `B-009` | upstream pre-merge PR gate plus merge wrapper/closure audit | pre-merge 无 dispatch 证据仍可通过；gate-before-review、dispatch-before-gate、head mismatch fixtures 失败 |
| `B-010` | upstream executable closure audit synced into remem | 缺少合规 review 链的 merged fixture 输出 machine-readable violation/required follow-up |
| `B-011` | protection status evidence、CONTRIBUTING | 无 protection 时仅报告 advisory；管理员设置由 live API/拒绝 merge 证据验证 |
| `B-012` | review artifact lifecycle/runtime ledger | cancelled、superseded 与并发 current-head 终态 fixtures |
| `B-013` | runtime ledger/PR evidence self-review recovery | 无 lane failure、无同 head 人类授权或缺 human final review 的 fixtures 失败 |

## 数据流

```text
preference + reinforcement/candidate state + suppression policy
  -> centralized fail-closed eligibility policy
  -> parameterized compiler query
  -> compiled rule or explicit ineligible result

reviewer lane
  -> schema-valid terminal review artifact bound to head SHA
  -> GitHub PR evidence + CI/thread evidence on same head
  -> pre-merge PR gate completes after review (no future dispatch required)
  -> human merge authorization / server-side required check
  -> merge wrapper records dispatch, or closure audit detects an external merge
  -> compliant merge evidence or machine-readable violation/follow-up
```

本变更不新增 remem 用户数据表。review artifact 的格式迁移发生在 SpecRail workflow
evidence 层；旧 artifact 只保留历史审计用途，不可用于新的 merge-ready 决策。

## 备选方案

- 固定完整 SQL 字符串：拒绝。它会因格式、列顺序或无语义重构产生噪声，也无法证明
  未知状态 fail closed；行为矩阵能更稳定地固定资格边界。
- 只更新 CONTRIBUTING：拒绝。文档不能验证审查终态、head 和时间顺序，也不能阻止
  调用方伪造 `review_source`。
- 只依赖本地 runtime ledger：拒绝。它能约束 agent，但外部 GitHub merge 可以绕过。
- 在 remem 直接修改 synced checks：拒绝。下一次同步会覆盖修改并造成来源漂移。
- 让 agent 自动批准或合并：拒绝。与 `workflow.yaml` 的权限边界冲突，也不构成独立审查。

## 风险

- Security：eligibility 漏项可能把未审查内容提升为自动强制行为；必须封闭 allowlist、
  参数化查询并对未知值 fail closed。
- Compatibility：上游 artifact schema 升级后，旧产物不能用于 merge-ready；需要明确错误，
  不做静默兼容。
- Performance：behavior matrix 只运行于测试；compiler 仍使用单次参数化查询，不在 hook
  热路径增加网络或 LLM 调用。
- Maintenance：上游 SpecRail 与 remem lock 可能短期不同步；同步验证必须阻止漂移。
- Governance：没有 GitHub 服务端 protection 时无法对管理员 merge 提供物理阻止；产品
  和日志必须如实暴露这一限制。

## 测试计划

- [ ] Rust focused tests：`cargo test rules::compiler -- --nocapture` 覆盖完整资格行为矩阵、
      unknown values 与关键交叉状态。
- [ ] SpecRail upstream tests：machine-readable enforcement classification、review artifact
      lifecycle、exact head、prior-findings carry-forward（含 unresolved 阻断）、完成时间、
      thread resolver role 与 successor re-review evidence、pre-merge no-dispatch 正例、
      post-dispatch/closure-audit 负例，以及受限 self-review 恢复路径。
- [ ] Sync verification：`scripts/sync-specrail-checks.sh --verify`。
- [ ] Workflow checks：`python3 checks/check_workflow.py --repo .` 与
      `python3 checks/check_workflow.py --repo . --spec-dir specs/GH813`。
- [ ] Repository gates：实现 PR 执行 `cargo fmt --check`、`cargo check`、focused tests 和
      `cargo test`；spec-only PR 执行 workflow checks 与 `git diff --check`。
- [ ] Human/admin verification：仅在决定启用服务端 protection 时，用 live GitHub API
      记录 ruleset/branch protection，并证明缺少 required check 的 merge 被拒绝。

## 回滚方案

- eligibility 实现回滚时保留 fail-closed：关闭新的编译路径或恢复前一版策略，不能放宽
  未知状态的资格。
- 上游 SpecRail 同步异常时，将 lock 恢复到上一已验证 SHA 并重新执行 `--verify`；旧格式
  不能临时作为新 merge-ready 证据。
- CONTRIBUTING/spec 可随实现回滚，但必须保留“没有服务端保护就只能 advisory”的事实。
- branch protection/ruleset 的变更由人类管理员单独回滚并记录，不与代码回滚混合。
