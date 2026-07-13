# Product Spec

## Linked Issue

GH-813

## 用户问题

GH-671 的 T3 实现链（#791 → #797 → #801）最终在 `main` 上达到了正确状态，
但经历了“先合并、后由独立审查发现问题、再补丁修复”的三轮过程。第一次合并时，
自动执行规则的资格边界没有被完整写成封闭契约；后续 PR 又能在独立审查尚未完成时
被外部合并。结果是高风险或未审查的偏好曾短暂具备被编译为自动规则的可能，且
仓库的本地流程无法证明审查在合并前已经完成。

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

- 不回滚 #791、#797 或 #801；当前 `main` 的 eligibility 实现视为正确基线。
- 不在本 spec PR 中修改运行时代码、GitHub 权限、branch protection 或 ruleset。
- 不允许 agent 获得最终批准、合并或仓库权限管理权。
- 不用脆弱的 SQL 文本或空白快照代替行为契约测试。
- 不把本门禁扩展为所有普通文档或低风险内部重构的通用流程。

## Behavior Invariants

1. `B-001`：任何把记忆、用户输入或提取内容提升为自动执行、自动阻止或确定性强制
   行为的变更，都必须标记为 `enforcement_sensitive`；实现开始前必须存在已批准的
   Product/Tech eligibility 契约。
2. `B-002`：eligibility 契约必须显式列出所有参与判定的维度和封闭 allowlist。对
   preference-rule compilation，唯一允许组合是：memory type=`preference`；status=
   `active` 且未过期；scope=`project` 时 owner=`repo` 且解析出的 target/owner/project 等于
   当前项目，或 scope=`global` 时 `owner_scope='user'`、`owner_key='user:default'` 且没有
   project target；source
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
6. `B-006`：独立审查只有在 reviewer lane 产生终态结果、绑定 final head SHA 且留下
   完整证据后才算完成。缺失、pending、零输出、取消、崩溃或证据读取失败都必须阻止
   merge-ready 状态，不能静默替换为 self-review。
7. `B-007`：PR head 变化后，旧 head 的审查结果立即失效；新的审查必须绑定新 head，
   并显式复核上一轮未解决 findings。
8. `B-008`：未解决的 reviewer 或 human actionable thread 阻止 merge-ready；agent
   不得代替 reviewer 或 human 将其标记为已解决。
9. `B-009`：merge gate 必须在独立审查完成之后、实际 merge dispatch 之前查询，且
   review、CI、thread 和 merge gate 都必须绑定同一 final head。对
   `enforcement_sensitive` PR 不存在 fast-path 例外。
10. `B-010`：如果外部或管理员在证据不完整时完成合并，closure audit 必须把它记录为
    gate violation 并创建或保留修复工作；不得仅因 PR 已合并就报告合规完成。
11. `B-011`：仓库必须如实记录服务端 branch protection/ruleset 是否存在。没有服务端
    required check 时，本地 gate 只能宣称“检测和阻止 agent 自主推进”，不能宣称
    “GitHub 上不可绕过”。启用或修改服务端保护只能由有权限的人类管理员执行并留证。
12. `B-012`：reviewer lane 的启动、完成、失败、取消和 superseded 状态必须可审计；
    并发审查只接受绑定当前 head 的唯一有效终态结果，过期结果不得覆盖新结果。
13. `B-013`：self-review 仅能作为已记录 reviewer-lane failure 的恢复路径，且必须有
    针对同一 PR 和 head 的独立人类授权。没有 lane failure、授权范围不匹配或授权缺失时
    必须阻止 merge-ready；self-review 不能替代 human final review。

## 验收标准

- [ ] GH-671 的 authoritative Product/Tech contract 显式枚举完整 eligibility 维度、
      allowlist 和 unknown-value fail-closed 行为。
- [ ] compiler 测试以一个完整正例、逐维反例和关键交叉状态反例覆盖 `B-002` 至
      `B-005`；candidate risk 与 reinforcement risk 可独立变异，closed enum 新值会让
      coverage 检查失败，且测试不依赖 SQL 文本格式。
- [ ] merge evidence 不能只接受调用方提供的 `review_source` 字符串；它必须读取经
      schema 校验、绑定 final head 且有完成状态/时间的独立审查产物。
- [ ] 自动化负例覆盖：审查仍在运行、审查失败或取消、审查为空、head 已变化、审查在
      merge 之后完成、存在 unresolved thread、审查产物不可读。
- [ ] `CONTRIBUTING.md` 说明 `enforcement_sensitive` 无 fast path，并明确 advisory
      gate 与服务端 protection 的能力边界。
- [ ] 仓库记录当前 branch protection/ruleset 状态；若需要不可绕过保护，由人类管理员
      完成 GitHub 设置并附验证证据。
- [ ] closure audit 能将“未满足 exact-head review gate 的已合并 PR”判定为违规并保留
      follow-up，而不是判定为成功闭环。

## 边界情况

- GitHub API 离线、限流或权限不足：无法取得审查或保护状态时必须阻止 merge-ready，
  并报告缺失的证据类型。
- reviewer lane 在运行中被取消、崩溃或返回空结果：记录失败终态，不降级为 self-review。
- reviewer lane 明确失败后请求 self-review：只有同一 PR/head 的独立人类授权存在时才可
  进入恢复路径，且 human final review 仍不可省略。
- 审查完成后 PR 新增 commit：旧审查标记为 superseded，重新审查新 head。
- 多个 reviewer lane 并发：仅接受绑定 current head 的有效终态；任一当前 actionable
  finding 未解决时仍阻止合并。
- 管理员直接合并：仓库内工具可能无法物理阻止，但 closure audit 必须可检测、记录并
  路由修复；只有服务端 required check/protection 才能提供不可绕过保证。
- 旧版审查产物：在迁移窗口内可读取用于历史审计，但不能作为新的 merge-ready 证据。

## 发布说明

该变更先以 spec 和流程门禁发布，不改变 remem 用户侧运行时行为。仓库内实现与上游
SpecRail 证据格式变更必须分别完成；同步文件只能在上游发布后通过既有同步流程进入
remem。GitHub branch protection/ruleset 属于独立的人类管理员动作，是否启用及验证
结果必须显式记录。spec 批准前不得开始实现。
