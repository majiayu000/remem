# Tech Spec

<!-- specrail-planned-changes
{"version":1,"issue":813,"complete":true,"paths":[".github/pull_request_template.md",".github/workflows/ci.yml",".github/workflows/closure-audit.yml",".github/workflows/sensitive-governance.yml","CHANGELOG.md","CONTRIBUTING.md","Cargo.lock","Cargo.toml","checks/check_workflow.py","checks/closure_audit.py","checks/duplicate_work_gate.py","checks/github_approved_spec_evidence.py","checks/github_duplicate_evidence.py","checks/github_evidence_common.py","checks/github_issue_evidence.py","checks/github_issue_reference.py","checks/github_pr_evidence.py","checks/github_pr_snapshot.py","checks/github_review_evidence.py","checks/pr_gate.py","checks/pr_review_contract.py","checks/rejection_items.py","checks/review_json_gate.py","checks/review_result_semantics.py","checks/route_gate.py","checks/runtime_budget_dimensions.py","checks/runtime_gate_rules.py","checks/runtime_ledger_gate.py","checks/schema_contract.py","checks/schema_validation.py","checks/sensitive_enforcement.py","checks/session_telemetry.py","checks/specrail-sync.lock.json","checks/specrail_lib.py","npm/remem/package.json","plugins/remem/.codex-plugin/plugin.json","plugins/remem/runtimes/remem-releases.json","schemas/closure_audit_result.schema.json","schemas/duplicate_work_evidence.schema.json","schemas/pr_review_gate.schema.json","schemas/review_result.schema.json","schemas/runtime_checkpoint.schema.json","schemas/sensitive_implement_gate_result.schema.json","scripts/ci/check_pr_tier.py","scripts/ci/closure_follow_up.py","scripts/ci/extract_nonclosing_issue.py","scripts/ci/run_sensitive_implement_gate.py","scripts/ci/test_closure_follow_up.py","scripts/ci/test_extract_nonclosing_issue.py","scripts/ci/test_run_sensitive_implement_gate.py","scripts/ci/test_schema_contract.py","scripts/ci/test_sensitive_governance_workflow.py","scripts/ci/test_specrail_gate_wiring.py","scripts/sync-specrail-checks.sh","server.json","specs/GH813/product.md","specs/GH813/tasks.md","specs/GH813/tech.md","src/rules/compiler.rs","src/rules/compiler/eligibility_tests.rs","src/rules/compiler/sweep_tests.rs","src/rules/compiler/tests.rs","workflow.yaml"],"spec_refs":["specs/GH813/product.md","specs/GH813/tech.md"]}
-->

## Linked Issue

GH-813

## Product Spec

Product: `product.md`

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Preference eligibility | `src/rules/compiler.rs`, `src/rules/compiler/tests.rs`, `src/rules/compiler/eligibility_tests.rs`, `src/rules/compiler/sweep_tests.rs` | SQL 已检查 lifecycle、expiry、scope、source trust、machine-checkable、reinforcement threshold/risk、candidate risk/review status 和 suppression policy；但 global scope 只要求 `owner_scope IS NOT NULL`，且测试 fixture 让两个 risk 共用字段。 | 需要修正现存 global-owner 过宽问题，把 eligibility 变成封闭、可枚举、fail-closed 的契约，并证明每个条件不可遗漏；#906 的完整 changed-path replay 必须覆盖这四个 compiler 文件。 |
| GH-671 contracts | `specs/GH671/product.md`, `specs/GH671/tech.md`, `docs/specs/preference-rule-compilation/PRODUCT.md`, `docs/specs/preference-rule-compilation/TECH.md` | 现有契约概括了 low-risk、trusted source、scope 和 machine-checkable，但没有枚举全部 review/trust/risk 允许值与交叉状态。 | `B-001` 至 `B-005` 要求 authoritative contract 与第一轮实现测试一致。 |
| Review artifact | `schemas/review_result.schema.json`, `checks/review_json_gate.py` | artifact 可带 `head_sha`、`review_round` 和 `mode`，但 schema/gate 不要求独立 reviewer 身份、开始/完成时间或终态。 | 仅有 PASS 文本不足以证明独立审查在合并前完成。 |
| PR evidence and gate | `checks/github_pr_evidence.py`, `checks/pr_gate.py` | evidence 接受调用方传入的 `review_source`；gate 检查 exact head、threads 与 merge ordering，但没有加载并验证对应的独立审查 artifact。 | 必须从可验证产物建立 review completion → gate → merge 的时序链。 |
| Runtime ledger | `schemas/runtime_checkpoint.schema.json`, `checks/runtime_ledger_gate.py` | queue ledger 已记录 reviewer lane 和 review source，并可阻止 agent 自主推进。 | 可复用 lane/失败状态语义，但不能把本地 ledger 误称为 GitHub 服务端保护。 |
| Synced SpecRail checks | `scripts/sync-specrail-checks.sh`, `checks/specrail-sync.lock.json` | review schema、GitHub evidence 和 PR gate 从上游 `majiayu000/specrail` 固定版本同步。 | remem 不得直接修改这些 vendored 文件；需要上游变更后再同步。 |
| Prospective implementation readiness | `scripts/ci/run_sensitive_implement_gate.py`, `schemas/sensitive_implement_gate_result.schema.json`, `.github/workflows/ci.yml`, repo-local workflow adapter | 上游 issue evidence 不包含 readiness label event 的 actor/time，duplicate-work evidence 也不绑定 repository/local remote；当前 implementation PR 及其 remote head branch 还会作为自身 duplicate 命中。 | remem-local wrapper 必须 live 绑定 repository、local remote、current PR/exact head，自行查询 readiness label event，验证 5 分钟 freshness，并仅对精确当前 PR/head 及其 live-verified 唯一 remote head branch 做可审计自引用 exemption；不能手改 vendored gate。 |
| Repository governance | `workflow.yaml`, `CONTRIBUTING.md`, `.github/workflows/sensitive-governance.yml`, `.github/workflows/closure-audit.yml` | workflow 禁止 agent 最终批准、合并和权限变更；普通 required status check 只按名称和 App 来源识别，不绑定 workflow/event，同仓库 GitHub Actions 不能成为不可伪造信任根。 | prospective workflow 从 GitHub API fresh 绑定 live default-branch snapshot；closure workflow 固定被审计 merge 的可信 pre-merge parent。两者只执行 trusted-base classifier/controller 并标记 advisory；不可绕过的 merge enforcement 需要独立 GitHub App expected source 或组织级 required workflow。 |
| Required source-version staging | `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `npm/remem/package.json`, `plugins/remem/.codex-plugin/plugin.json`, `plugins/remem/runtimes/remem-releases.json`, `server.json` | 仓库的 version-bump 与 plugin-version-sync gates 要求用户可见 Rust 行为变更同步提升并对齐所有发布元数据。 | 本 issue 的 compiler correctness fix 必须只做一次一致的 unreleased source-version staging；这些文件不得承载额外产品行为或发布动作。 |

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

以下文件由 `checks/specrail-sync.lock.json` 管理，必须先在上游 SpecRail 实现。remem
通常同步正式 release；当 release 明显落后且维护者明确授权时，也可以固定经过验证的
exact commit SHA。两种路径都必须由 `scripts/sync-specrail-checks.sh` 从 clean upstream
checkout 复制并验证内容哈希，禁止手工修改 vendored 文件：

- 扩展 review artifact，至少要求 `reviewer_lane`（或等价独立身份）、`head_sha`、
  `review_started_at`、`review_completed_at`、终态、verdict 和结构化 findings。只有显式
  clean/non-blocking verdict 且 current-head artifact 中没有 blocking/actionable finding 时，
  review evidence 才能参与 merge-ready 判定；`changes_requested`、任何 blocking verdict，
  或仅存在于 artifact 而没有对应 GitHub thread 的 current-head actionable finding 都必须
  fail closed。取消、失败、空输出与 superseded 必须是可区分状态。
- review artifact 对每个 superseded head 携带 `prior_findings`：稳定 finding ID、来源
  head、`resolved|unresolved|obsolete` 状态及 resolved/obsolete 证据。gate 读取上一有效
  artifact 并验证新 artifact 完整覆盖旧 findings；缺项、重复/冲突状态、无关闭证据或
  unresolved 均 fail closed。
- `review_json_gate` 校验终态、时间顺序、非空结果、exact head、verdict allowlist 和
  current-head findings rollup，而不只校验 verdict 文本；thread rollup clean 不能覆盖
  artifact 自身的 blocking/actionable finding。
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
  head；外部 merge 缺少该链时输出 schema-valid machine-readable violation 和
  `required_follow_up` payload。上游 `SP813-T3` 只拥有检测与 payload contract，不直接写
  remem 的 GitHub issue。
- self-review evidence 只有在存在可验证的 reviewer-lane failure、同一 PR/head 的独立
  人类授权且后续仍有 human final review 时才有效；否则 fail closed。
- thread rollup 对 resolved actionable reviewer/human thread 保留 resolver identity、
  verified role 和授权来源；只有原 reviewer、带可验证 re-review evidence 的 successor
  reviewer lane 或获授权 human maintainer 可清除阻塞。resolver 缺失、unknown、
  implementer、orchestrator 或 coordinator 的 resolved thread 仍 fail closed。
- 增加 fixture：pending、failed、cancelled、empty、`changes_requested`、其他 blocking
  verdict、current-head artifact-only actionable finding、stale head、review-after-merge、
  prior finding 缺失/无状态/状态仍为 unresolved/无 resolved-or-obsolete 证据、
  unresolved thread、由 implementer/orchestrator/coordinator/unknown resolver 解决的
  actionable thread、无 re-review evidence 的 successor reviewer resolver、artifact
  unreadable、伪造 source、无授权 self-review、缺失/冲突 enforcement classification、
  pre-merge 缺失未来 dispatch 的正例、dispatch-before-gate，以及外部 merge 无证据链。

### 3. remem 同步与仓库流程说明

- 上游变更发布后，或维护者明确批准 exact-SHA pin 后，更新
  `checks/specrail-sync.lock.json` 并通过同步脚本更新 vendored 文件；禁止在 remem 内对
  synced 文件做永久性手改。exact-SHA 例外必须由同仓库 durable maintainer comment
  记录 actor、完整 SHA、授权范围与时间；实现 PR 必须引用该 URL。当前 sync/route gate
  不解析 GitHub comment，也不自动证明授权；human final review 必须 live read-back URL，
  人工核对 actor/scope/time/full-SHA 与 lock 完全一致，sync gate 只负责验证 lock SHA、
  tracked-file closure 和 content hashes。2026-07-21 维护者在
  `https://github.com/majiayu000/remem/issues/813#issuecomment-5030044760`
  批准固定 `0f903abe1794899071a9f19a4c46af1ce81129d3`。以下是维护者声明、必须在 human final
  review 重新 live 验证的外部证据：同日
  `gh api repos/majiayu000/specrail/compare/v0.2.1...main` 返回 `ahead_by=168`；上游
  PR #103/#115/#116 均为 merged 且各自 `workflow-check` SUCCESS；clean checkout 在该
  exact SHA 执行 T3 指定的五个 pytest 文件，结果为 `254 passed in 7.12s`。
- 该 pin 已有必须保留的失败历史：PR #892 首次同步后因 remem 本地 schema-contract
  断言与 upstream `SchemaDefinitionError` 合同不一致而使 main CI 失败，PR #893 紧急
  revert。GH-894 的 spec PR #895 明确修复边界，PR #897 exact head
  `5ca704454e618473e74ebb6c8c15ecbf77158797` 适配本地 schema-contract、重新同步同一 SHA，
  maintainer-asserted CI run `29683610679` 的 `check` SUCCESS 后合并。当前授权确认的是 #897 已修复并验证的
  re-land 状态，不追溯性把 #892 描述为合规。
- 在 `CONTRIBUTING.md` 增加 `enforcement_sensitive` 分类、无 fast path、exact-head review
  顺序和 agent 权限边界。避免修改 `AGENTS.md`，除非维护者另行要求高上下文规则变更。
- 同步并接入上游可执行 closure audit；`SP813-T5` 的 remem workflow integration controller
  是 durable follow-up 的实现 owner。它在已合并 PR 缺少合规 review completion 链时消费
  上游 `required_follow_up` payload，通过 GitHub Issues API 创建、重新打开或复用一个
  durable issue，并以 `repository + pr_number + final_head_sha + violation_code` 作为稳定
  幂等键。controller 必须回读 issue number、URL、open state 和幂等键，并把它们写回 closure
  evidence；只有本地产物而没有可回读的 issue/queue item 不算 follow-up 已保留。GitHub API
  不可用、权限不足或写入/回读失败时必须返回 error/blocked，不能把 artifact 当作持久化成功。
- `.github/workflows/sensitive-governance.yml` 使用 `pull_request_target` 和只读权限；每次运行
  都通过 GitHub API fresh 查询 live `defaultBranchRef.target.oid`/base-ref snapshot，并 checkout
  该 exact SHA。PR payload 的 `base.sha` 只作为被比较的快照，不得作为 trusted-code checkout；
  如果 live default branch、base ref 或 PR snapshot 无法一致解释，workflow fail closed。changed
  paths 只从 GitHub API 读取，分类器、registry 与依赖全部来自 live trusted base，绝不 checkout、
  import 或执行 PR head。它只生成 `advisory_only` artifact，
  普通 PR CI 不能作为最终治理授权。`.github/workflows/closure-audit.yml` 同样固定
  pre-merge `base.sha` 后再分类和运行 controller，避免 merged head 缩减 registry 或替换
  controller；但目标 PR 仍可能删除 repo-local workflow、阻止 closed event dispatch，因此
  两个 workflow 都不是 T6 外部信任根。

### 4. GitHub 服务端保护的人类边界

- 记录 `main` 的 branch protection/ruleset 查询结果和时间。
- 普通 required status check 不区分 workflow、matrix 或 event；若 expected source 是 GitHub
  Actions，目标 PR 可以产生同名 check。因此 selected required-check ruleset 只能降低误绕过
  风险，不能单独满足“外部或管理员也不能绕过”。
- 不可绕过的个人仓库方案由有权限的人类管理员安装独立 GitHub App，让 App 在仓库外持有
  policy/controller 并发布专用 check，再由 ruleset 把 expected source 固定到该 App；若迁入
  组织，也可用受保护治理仓库中的 organization required workflow。两种方案都必须验证缺少
  可信来源 gate 时 merge 被拒绝，且外部 closure audit 不受目标 PR 删除 workflow 影响。
- agent 不修改权限、不批准、不合并。服务端保护未启用时，工具必须把能力描述为
  advisory detection，而不是 prevention。
- 2026-07-21 live API 返回 `main` 未受 branch protection/ruleset 保护；维护者已选择
  required-check ruleset：要求 `check` 成功、解决 review conversations、禁止 force push
  与 deletion，且不要求第二位 approver 以避免单维护者仓库自锁。独立审查确认 GitHub
  Actions-sourced `check` 不能绑定特定 workflow/event，因此该规则只能作为 advisory/
  accidental-bypass mitigation；T6 仍需独立 App expected source 或组织 required workflow。

### 5. prospective implement gate wrapper

- `SP813-T5` 在 remem 本地新增 `scripts/ci/run_sensitive_implement_gate.py` 及其
  focused test；它不是 synced SpecRail 文件，不修改 lock 管理的 gate。wrapper 必须接收
  `--github-repo <owner/name> --issue <number> --pr <number> --head-sha <full-sha>`，验证 local
  `origin` remote 与 repository 一致，并通过 live GitHub API 验证该 open PR 属于同一仓库、
  非 fork、head repository/ref/remote-tracking commit 精确匹配且引用该 issue。它自行查询
  issue label timeline，要求 maintainer readiness
  label event 的 actor/time 非空，并通过 live GitHub repository permission/role API 证明 actor
  类型是 `User`（拒绝 Bot/App/agent），且为 repository owner 或拥有 `admin|maintain` 权限；
  durable result 保存 actor type、permission/role 与 authority query source，并与
  `state_source=label`、`state_trusted=true` 一致；不得声称
  `github_issue_evidence.py` 提供其 schema 中不存在的 actor/time。wrapper 调用并校验
  `github_issue_evidence.py` 与 `github_duplicate_evidence.py` 的 live 产物，记录 repository、
  remote URL，并拒绝未来时间、不可解析时间或 gate 开始时已超过 300 秒的 `collected_at`。
- duplicate collector 的原始 artifact 必须完整保留。wrapper 只可复制并过滤出经上述 live
  校验的当前 PR number + exact head 自引用，以及该 PR API payload 的 head ref + exact head
  唯一确定的 remote head branch 自引用；把 exemption 的 repository/PR/head/head-ref、原始和
  过滤后 PR/branch artifact hashes 写入 durable result。其他引用该 issue 的 PR 或匹配
  remote branch 仍阻断。
  synced route gate 只消费过滤后副本，wrong repository/remote/head 或无法证明唯一自引用时
  fail closed。
- wrapper 对 issue evidence 与 duplicate-work evidence 分别计算 pre-gate SHA-256，再用
  数组参数固定调用 `route_gate.py --route implement --issue <issue> --mode required
  --evidence <issue-evidence> --duplicate-evidence <duplicate-evidence> --json`；wrapper API 不接受
  `--state` 或 `--label`，也不得把它们加入子进程。这里的 duplicate path 必须是上段定义的
  过滤后副本；gate 完成后立即重算 route 输入的两份文件 hash，任一
  前后不一致即 fail closed。durable result 必须保存两份 pre/post hash、完整固定 argv、
  nested gate JSON，并验证 wrapper result 中 `state_trusted=true`、nested gate 的
  `current_state=ready_to_implement` 与 `satisfied` 明示 state 来自 evidence、
  `decision=allowed`、`missing=[]`、
  `sensitive_classification.enforcement_sensitive=true`；任一步骤 error/blocked 都不得降级。
- `schemas/sensitive_implement_gate_result.schema.json` 要求 wrapper identity、current head、
  started/completed timestamps、固定 argv、两份 evidence 的 pre/post hashes、wrapper trust/
  freshness verdict 和 nested route-gate JSON。CI 与 repo-local PR evidence adapter 只接受该
  schema 的 current-head artifact；裸 `route_gate.py` 输出缺少 wrapper identity/hash chain，
  即使 `decision=allowed` 也必须拒绝。`.github/workflows/ci.yml` 只能调用 wrapper，
  `CONTRIBUTING.md` 与 PR template 明确禁止把直接 gate 调用作为 sensitive readiness 证明。
- duplicate evidence 除已验证 current PR/exact-head 及其唯一 remote head branch 自引用外
  命中实现 PR，或命中其他匹配远端分支时，保存原始 conflict artifact。
  解除 blocker 必须引用 human decision 的 actor/time/rationale；如决定清理分支，还必须
  同时保存 cleanup 前后 evidence 和 decision URL。agent 无权删除冲突分支来获得绿灯。
- 输出 `allowed` 前，wrapper 必须重新 live 查询 PR open/exact head/ref/body/repository，并重新读取
  当前 active `ready_to_implement` label interval 的最新 event。末次 event 必须与前次保存的 event
  identity 一致，并重新通过 actor type、owner/admin/maintain permission 与 authority-source 校验；
  label 被撤销、由低权限 actor 重新添加、event 漂移，或任一 PR head/state/body 漂移都 fail closed。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001` | workflow/checkpoint/PR-evidence schema、sensitive registry、route/pr gates | 缺失标记、registry 冲突、sensitive 无批准 spec 的 fixtures 均失败；有效 true + approved spec 通过 |
| `B-002` | compiler eligibility policy、GH-671 specs | 完整 eligible fixture；逐维 allowlist 表与测试集合一致 |
| `B-003` | compiler query/policy | table-driven 单变量反例全部不产生 compiled rule |
| `B-004` | compiler tests | 正例、逐维反例和关键交叉反例在同一实现 PR 通过 |
| `B-005` | typed parsing/query policy | unknown trust/risk/review/scope/lifecycle fixtures 均 ineligible |
| `B-006` | upstream review schema/json gate/evidence | 只有 clean/non-blocking verdict 且 current-head 无 blocking/actionable finding 才可进入 merge-ready；pending、failed、cancelled、empty、unreadable、`changes_requested`、其他 blocking verdict 和 artifact-only actionable finding fixtures 均失败 |
| `B-007` | upstream review artifact/evidence/pr gate | stale-head、新 head 遗漏 prior finding、carry-forward 后仍 unresolved、resolved/obsolete 无证据 fixtures 失败；完整 carry-forward + 有关闭证据 + exact-head review 通过 |
| `B-008` | upstream PR evidence/thread rollup | unresolved reviewer/human thread、implementer/orchestrator/coordinator/unknown resolver，以及 successor reviewer 缺 re-review evidence 的 fixtures 失败；原 reviewer、有 re-review evidence 的 successor reviewer lane、获授权 human maintainer resolver 通过 |
| `B-009` | upstream pre-merge PR gate plus merge wrapper/closure audit | pre-merge 无 dispatch 证据仍可通过；gate-before-review、dispatch-before-gate、head mismatch fixtures 失败 |
| `B-010` | `SP813-T3` upstream closure-audit payload + `SP813-T5` remem workflow integration controller | merged violation 先输出 schema-valid `required_follow_up`；controller 按稳定幂等键创建/复用并回读 open GitHub issue。写入/回读失败和 artifact-only follow-up fixtures 均阻止 closure；重复运行复用同一 issue |
| `B-011` | protection status/source evidence、CONTRIBUTING、trusted-base advisory workflows | repo-local/GitHub Actions check 仅报告 advisory；独立 App expected source 或 org required workflow 由 live API、来源绑定与拒绝 merge 证据验证 |
| `B-012` | review artifact lifecycle/runtime ledger | cancelled、superseded 与并发 current-head 终态 fixtures |
| `B-013` | runtime ledger/PR evidence self-review recovery | 无 lane failure、无同 head 人类授权或缺 human final review 的 fixtures 失败 |
| `B-014` | remem-local sensitive implement wrapper/result schema + live GitHub repository/PR/label timeline + issue/duplicate evidence + synced route gate | 裸 route JSON、wrong repository/remote/PR/head/head-ref、fork、同名 remote branch SHA 不匹配、未来/不可解析/超过 300 秒的 `collected_at`、非 trusted label、缺 label-event actor/time、Bot/App/agent labeler、无 maintain/admin 权限的 labeler、不完整 PR 列表、非当前 PR 冲突、其他匹配 branch、过宽 self-exemption、state/label override、gate 期间任一 evidence hash 改变、final PR 漂移、readiness label 撤销、active label event 被低权限 actor 替换或 event identity 漂移，以及无 durable human decision 的 cleanup fixtures 失败；schema-valid current-head wrapper result、固定 argv、repository/headRepository/remote commit binding、首末两次一致且均通过权限校验的 live human-maintainer label event 与 authority source、仅当前 PR/head 及其 live-verified 唯一 remote head branch exemption、原始/过滤后 PR/branch artifact hashes 与 trusted evidence state 的正例通过 |

## 数据流

```text
preference + reinforcement/candidate state + suppression policy
  -> centralized fail-closed eligibility policy
  -> parameterized compiler query
  -> compiled rule or explicit ineligible result

reviewer lane
  -> schema-valid terminal clean/non-blocking review artifact bound to head SHA
     with zero current-head blocking/actionable findings
  -> GitHub PR evidence + CI/thread evidence on same head
  -> pre-merge PR gate completes after review (no future dispatch required)
  -> human merge authorization / external-App-or-org-required-workflow trust root
  -> merge wrapper records dispatch, or closure audit detects an external merge
  -> compliant merge evidence or machine-readable violation/required_follow_up payload
  -> remem workflow integration controller creates/reuses and reads back a durable GitHub issue
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
      lifecycle、clean/non-blocking verdict、current-head zero-blocking-findings、exact head、
      prior-findings carry-forward（含 unresolved 阻断）、完成时间、
      thread resolver role 与 successor re-review evidence、pre-merge no-dispatch 正例、
      `changes_requested`/blocking verdict/artifact-only actionable finding 负例、
      post-dispatch/closure-audit 负例，以及受限 self-review 恢复路径。
- [ ] Durable follow-up integration：外部违规 merge 的 `required_follow_up` 通过 remem
      controller 创建/复用并回读 open GitHub issue；重复运行保持同一幂等键和 issue，API
      写入/回读失败或只有 artifact 时 closure 保持 blocked。
- [ ] Sync verification：`scripts/sync-specrail-checks.sh --verify`。
- [ ] Workflow checks：`python3 checks/check_workflow.py --repo .` 与
      `python3 checks/check_workflow.py --repo . --spec-dir specs/GH813`。
- [ ] Repository gates：实现 PR 执行 `cargo fmt --check`、`cargo check`、focused tests 和
      `cargo test`；spec-only PR 执行 workflow checks 与 `git diff --check`。
- [ ] Human/admin verification：二选一并留下 durable 管理员证据：若启用不可绕过保护，
      用 live GitHub API 记录 ruleset/branch protection、required check expected source 与外部
      App/org required workflow 状态，并证明缺少可信来源 gate 的 merge 被拒绝；若本期保持
      advisory-only，则明确记录该降级决策、能力边界与后续重新评估条件，不要求伪造不存在的
      外部 trust-root 或 rejected-merge 证据。

## 回滚方案

- eligibility 实现回滚时保留 fail-closed：关闭新的编译路径或恢复前一版策略，不能放宽
  未知状态的资格。
- 上游 SpecRail 同步异常时，将 lock 恢复到上一已验证 SHA 并重新执行 `--verify`；旧格式
  不能临时作为新 merge-ready 证据。
- CONTRIBUTING/spec 可随实现回滚，但必须保留“repo-local 或 GitHub Actions 同源 status check
  只能 advisory，外部 App/org required workflow 才是不可伪造信任根”的事实。
- branch protection/ruleset 的变更由人类管理员单独回滚并记录，不与代码回滚混合。
