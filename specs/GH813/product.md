# Product Spec

## Linked Issue

GH-813

## 用户问题

GH-671 的 T3 实现链（#791 → #797 → #801）修复了 issue 已记录的三类问题，
但经历了“先合并、后由独立审查发现问题、再补丁修复”的三轮过程。第一次合并时，
自动执行规则的资格边界没有被完整写成封闭契约；后续 PR 又能在独立审查尚未完成时
被外部合并。结果是高风险或未审查的偏好曾短暂具备被编译为自动规则的可能，且
仓库的本地流程无法证明审查在合并前已经完成。把资格边界写成封闭契约后又暴露出一个
现存缺口：global 分支只要求 `owner_scope` 非空，仍可能接受 malformed/legacy owner，
因此 #813 也必须交付这一小型 correctness fix，不能只做漂移预防。

对“把记忆内容提升为自动执行或自动阻止行为”的功能，这类缺口会直接降低用户对
记忆系统的信任。资格边界、独立审查和合并证据必须在第一次实现中同时成立，而不是
依赖合并后的补丁来收敛。

## 目标

- 为所有会把存储内容提升为自动执行行为的变更定义统一的
  `enforcement_sensitive` 分类和前置门禁。
- 把 eligibility 从开放式实现细节提升为显式、封闭、可测试的产品契约；未知状态默认
  不具备资格。
- 要求合并证据证明独立审查已在同一 final head 上完成，且完成时间早于 merge gate
  和实际合并。
- 明确区分仓库内的 advisory 检查与 GitHub 服务端的不可绕过保护，禁止把前者描述为
  后者。
- 对绕过门禁的外部合并保留可审计的违规记录和修复队列，不把“已合并”当作“合规”。

## 非目标

- 不回滚 #791、#797 或 #801 已修复的 risk、directionality 和 override 语义；只针对新
  发现的 global-owner 过宽问题做 fail-closed 修正。
- 不在本 spec PR 中修改运行时代码、GitHub 权限、branch protection 或 ruleset。
- 不允许 agent 获得最终批准、合并或仓库权限管理权。
- 不用脆弱的 SQL 文本或空白快照代替行为契约测试。
- 不把本门禁扩展为所有普通文档或低风险内部重构的通用流程。

## Behavior Invariants

1. `B-001`：任何把记忆、用户输入或提取内容提升为自动执行、自动阻止或确定性强制
   行为的变更，都必须在 machine-readable workflow/PR evidence 中标记为
   `enforcement_sensitive`；实现开始前必须存在已批准的 Product/Tech eligibility 契约。
   缺少标记、与敏感 spec/path registry 冲突，或标记为 sensitive 却无批准契约时，route/
   PR gate 必须阻止推进。
2. `B-002`：eligibility 契约必须显式列出所有参与判定的维度和封闭 allowlist。对
   preference-rule compilation，唯一允许组合是：memory type=`preference`；status=
   `active` 且未过期；scope=`project` 时 `owner_scope='repo'`，并按
   `target_project → owner_key → legacy project` 取第一个非空值后等于当前项目；scope=
   `global` 时 `owner_scope='user'`、`owner_key='user:default'` 且没有 project target；source
   trust 属于 `local_tool_output|repo_file|user_prompt`；machine-checkable=true；
   reinforcement 达到阈值；reinforcement risk 和 originating candidate risk 分别为
   `low`；candidate review status 属于 `approved|edited|auto_promoted`；policy evaluation
   成功且不存在匹配 memory/topic-key/entity/pattern 的 `active` suppression。两个 risk
   来源必须独立满足，不能由同一个测试字段或其中一个条件代替另一个。
3. `B-003`：eligibility 是合取关系；任一条件不满足时，即使其余条件全部满足，内容也
   不得进入自动强制行为。
4. `B-004`：第一次实现 PR 必须同时包含一个完整正例、每个 eligibility 维度的独立
   反例，以及关键交叉状态反例。测试必须固定可观察行为，不能只固定 SQL 字符串。
5. `B-005`：新增或未知的 memory type、owner、trust、risk、review status、scope、
   lifecycle 或 policy 值，以及 malformed suppression state，默认 ineligible。实现必须
   以 closed-enum coverage 或等价完整性检查保证新 variant 先使测试/编译失败；要使其
   eligible，必须在同一 PR 中更新 Product/Tech 契约、实现和测试。
6. `B-006`：独立审查只有在 reviewer lane 产生终态结果、绑定 final head SHA、留下
   完整证据、给出显式 clean/non-blocking verdict，且 current-head artifact 没有 blocking/
   actionable finding 时才算完成。缺失、pending、零输出、取消、崩溃、证据读取失败、
   `changes_requested`、其他 blocking verdict，或仅存在于 current-head artifact 而没有对应
   GitHub thread 的 actionable finding 都必须阻止 merge-ready，不能静默替换为 self-review。
7. `B-007`：PR head 变化后，旧 head 的审查结果立即失效；新的审查必须绑定新 head，
   并携带所有上一 head findings 的稳定标识及 `resolved|unresolved|obsolete` 状态。缺项、
   无法证明 resolved/obsolete，或仍为 unresolved 时都阻止 merge-ready，不能因新审查
   没有复述旧 finding 就丢失阻塞项。
8. `B-008`：未解决的 reviewer 或 human actionable thread 阻止 merge-ready；已解决的
   actionable thread 只有在 resolver 被独立识别为原 reviewer、带可验证 re-review
   evidence 的 successor reviewer lane，或获授权 human maintainer 时才算清除。resolver
   缺失、未知、implementer、orchestrator 或 coordinator 时仍阻止 merge-ready；agent
   不得代替 reviewer 或 human 将其标记为已解决。
9. `B-009`：pre-merge gate 必须在独立审查完成之后、实际 merge dispatch 之前完成，且
   review、CI、thread 和 gate 都必须绑定同一 final head。merge-ready 判断不要求尚未产生
   的 dispatch 证据；merge wrapper 或 closure audit 在 dispatch 后验证 gate completion
   早于 dispatch。对 `enforcement_sensitive` PR 不存在 fast-path 例外。
10. `B-010`：如果外部或管理员在证据不完整时完成合并，closure audit 必须把它记录为
    gate violation 并创建或保留修复工作；不得仅因 PR 已合并就报告合规完成。
11. `B-011`：仓库必须如实记录服务端 branch protection/ruleset 和 check 信任来源。普通
    required status check 不区分 workflow 或 event；当 expected source 仍是 GitHub Actions
    时，本地 gate 只能宣称“检测和阻止 agent 自主推进”及降低误绕过风险，不能宣称
    “GitHub 上不可绕过”。不可绕过的 T6 信任根必须是 ruleset 绑定的独立 GitHub App，或
    组织级 required workflow 的受保护治理仓库。启用或修改这些保护只能由有权限的人类
    管理员执行并留证。
12. `B-012`：reviewer lane 的启动、完成、失败、取消和 superseded 状态必须可审计；
    并发审查只接受绑定当前 head 的唯一有效终态结果，过期结果不得覆盖新结果。
13. `B-013`：self-review 仅能作为已记录 reviewer-lane failure 的恢复路径，且必须有
    针对同一 PR 和 head 的独立人类授权。没有 lane failure、授权范围不匹配或授权缺失时
    必须阻止 merge-ready；self-review 不能替代 human final review。
14. `B-014`：任何新的 `enforcement_sensitive` 实现开始前，required implement route 必须
    只能通过 remem-local sensitive-implement wrapper 推进；CI/PR evidence 不接受裸
    `route_gate.py` JSON。wrapper 必须绑定并验证 live GitHub repository、local `origin`
    remote、当前 implementation PR number 与 exact head；PR 必须仍为 open、来自同一 repository
    而非 fork，且 head repository/ref/remote-tracking commit 必须与 exact head 一致。wrapper
    自行查询 maintainer readiness label event，并通过 live repository permission/role evidence
    证明 actor 是非 bot/app 的 owner、
    admin 或 maintain 级人类维护者；持久化 actor/timestamp/type/authority source，再要求
    `state_source=label`、`state_trusted=true`，以及
    collection age 不超过 5 分钟的完整 duplicate-work evidence；固定 route argv 不得包含
    `--state`/`--label`，两份 evidence 的 gate 前后 SHA-256 必须分别一致。wrapper 只可从
    route 输入副本中排除已通过 live API 验证的当前 implementation PR + exact head 自引用，
    以及同一 PR API payload 证明指向该 exact head 的唯一 remote head branch；并必须保留
    原始/过滤后 PR 与 branch evidence、各自 hash 与 exemption 记录。任何其他 PR、匹配远端
    实现分支、repository/remote mismatch、过期/不完整 evidence 都必须阻断。如果人类决定清理
    冲突分支或继续既有工作，冲突前后 evidence、决策 actor/time/rationale 必须持久保存，
    agent 不得自行删除分支来制造通过结果。wrapper 输出 `allowed` 前必须再次 live 查询并验证
    PR open state、repository/head repository、head ref、exact head、body 中的 issue/sensitive 声明，
    以及当前 active readiness label interval 的最新 event；所有 PR identity/body 值必须与首次
    保存值一致，末次 label event identity 也必须与首次保存值一致，
    并重新验证 actor type、owner/admin/maintain 权限和 authority source；label 撤销后由其他
    actor 重加、event identity 漂移、权限或 authority source 变化都必须 fail closed。

## 验收标准

- [ ] GH-671 的 authoritative Product/Tech contract 显式枚举完整 eligibility 维度、
      allowlist 和 unknown-value fail-closed 行为。
- [ ] compiler 测试以一个完整正例、逐维反例和关键交叉状态反例覆盖 `B-002` 至
      `B-005`；candidate risk 与 reinforcement risk 可独立变异，closed enum 新值会让
      coverage 检查失败，且测试不依赖 SQL 文本格式。
- [ ] merge evidence 不能只接受调用方提供的 `review_source` 字符串；它必须读取经
      schema 校验、绑定 final head、有完成状态/时间、显式 clean/non-blocking verdict 且
      current-head 无 blocking/actionable finding 的独立审查产物。
- [ ] workflow/check schema 存储 machine-readable `enforcement_sensitive` 分类；route/
      PR gate tests 证明敏感 registry 命中但标记缺失/为 false、标记为 true 但无批准 spec
      时都 fail closed。
- [ ] 自动化负例覆盖：审查仍在运行、审查失败或取消、审查为空、`changes_requested`、
      其他 blocking verdict、current-head artifact-only actionable finding、head 已变化、
      上一 head finding 未携带、carry-forward 后仍为 unresolved，或缺少 resolved/obsolete
      证据、审查在 merge 之后完成、存在 unresolved thread、actionable thread 被
      implementer/orchestrator/coordinator/unknown resolver 标记 resolved、审查产物不可读。
- [ ] `CONTRIBUTING.md` 说明 `enforcement_sensitive` 无 fast path，并明确 advisory
      gate 与服务端 protection 的能力边界。
- [ ] 仓库记录当前 branch protection/ruleset 状态和 required check 的 expected source；
      若需要不可绕过保护，由人类管理员配置独立 GitHub App 或组织级 required workflow
      信任根并附拒绝绕过的验证证据。
- [ ] closure audit 能将“未满足 exact-head review gate 的已合并 PR”判定为违规并保留
      follow-up，而不是判定为成功闭环。
- [ ] prospective sensitive-implementation CI/PR evidence 只接受 schema-valid、绑定 live
      repository/local remote/current same-repository non-fork PR/current head 的 wrapper durable
      result，拒绝裸
      route-gate JSON；wrapper 自行查询 maintainer-trusted readiness label event 的 actor/time，
      并以 live permission/role evidence 拒绝 bot/app、agent 或无 maintain/admin 权限的 labeler，
      验证 duplicate-work schema/完整性/5 分钟 freshness、无 state/label override 和 evidence
      的 pre/post hash 稳定性。仅允许精确当前 PR/head 及其 live-verified 唯一 remote head
      branch 的显式自引用 exemption，并保留原始/过滤后 PR/branch artifacts 与 hashes；
      wrong repository/remote、stale、其他 conflicting PR/branch、
      hash drift 或未保留的人类 ownership/cleanup 决策均 fail closed。输出 `allowed` 前必须
      重新读取并比对 PR open state、repository/head repository、head ref、exact head、body 中的
      issue/sensitive 声明和 active readiness label event；任何 PR identity/body 漂移均阻断，且
      必须证明首末 label event identity 一致、末次 actor type、
      owner/admin/maintain 权限与 authority source 仍有效；撤销/重加、event drift 或末次
      authority 校验失败均 fail closed。

## 边界情况

- GitHub API 离线、限流或权限不足：无法取得审查或保护状态时必须阻止 merge-ready，
  并报告缺失的证据类型。
- reviewer lane 在运行中被取消、崩溃或返回空结果：记录失败终态，不降级为 self-review。
- reviewer lane 明确失败后请求 self-review：只有同一 PR/head 的独立人类授权存在时才可
  进入恢复路径，且 human final review 仍不可省略。
- 审查完成后 PR 新增 commit：旧审查标记为 superseded，重新审查新 head。
- 旧 head 有 findings：新 head artifact 必须逐项携带稳定 finding ID、旧 head 和
  `resolved|unresolved|obsolete` 状态；resolved/obsolete 要有可验证证据，不能省略。
- 多个 reviewer lane 并发：仅接受绑定 current head 的有效终态；任一当前 actionable
  finding 未解决时仍阻止合并。
- actionable thread 显示 resolved：thread rollup 必须验证 resolver 身份和角色；原
  reviewer、带可验证 re-review evidence 的 successor reviewer lane 或获授权 human
  maintainer 之外的 resolver 不能清除阻塞状态。
- 管理员直接合并：仓库内工具可能无法物理阻止，但 closure audit 必须尽可能检测、记录
  并路由修复；只有 ruleset 绑定的独立 GitHub App 或组织级 required workflow 才能为本
  contract 提供不可由目标 PR 同名 check 冒充的信任根。
- duplicate-work evidence 收集后超过 5 分钟、系统时钟异常或无法解析 `collected_at`：
  implementation wrapper 必须报错并重新收集，不能继续使用旧 artifact。
- live evidence 除已验证的当前 PR/exact-head 及其唯一 remote head branch 自引用外发现已有
  实现 PR，或发现其他匹配远端分支：
  保留冲突 artifact；只有人类 ownership
  决策及其 actor/time/rationale 可解除，agent 不得静默删分支后重跑。
- 旧版审查产物：在迁移窗口内可读取用于历史审计，但不能作为新的 merge-ready 证据。

## 发布说明

该变更先以 spec 和流程门禁发布，不改变 remem 用户侧运行时行为。仓库内实现与上游
SpecRail 证据格式变更必须分别完成；同步文件通常跟随上游 release，在 release 明显落后
且维护者明确授权时允许固定经过验证的 exact commit SHA，但仍必须通过既有同步流程和
内容哈希验证进入 remem。exact-SHA 例外必须以同仓库的 durable maintainer evidence
记录 actor、完整 SHA、授权范围和时间，并由实现 PR 引用；不能只依赖本地对话或未绑定
SHA 的口头批准。当前 gate 不自动解析 GitHub comment；human final review 必须 live
read-back 并人工核对 actor/scope/time/full-SHA，sync gate 只验证 lock SHA 与 hashes。
2026-07-21 维护者声明已在 GH-813 durable comment
`issuecomment-5030044760` 授权固定
`0f903abe1794899071a9f19a4c46af1ce81129d3`，并选择 required-check ruleset。该授权只对
此 SHA 有效，不能复用于其他 upstream head。GitHub
branch protection/ruleset 的实际设置和 live 验证仍属于独立的人类管理员动作，不能由
agent 代替。
