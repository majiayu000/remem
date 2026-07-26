# Product Spec：CurrentTruth 统一读取契约

## Linked Issue

GH-933

PR #939 仅交付 Phase A 的只读 projection，使用 `Refs #933`。它是本产品
契约的 partial implementation，不关闭 GH-933，也不表示 Phase B 或 Phase C
已经完成。

## 用户问题

remem 已经保存 evidence、memory、observation、user-context claim、relation
等多种知识对象，但它们使用不同的状态与时间语义。调用方目前难以稳定回答：
在指定时间和 scope 内，系统认为哪些 claim 当前成立、哪些互相冲突、为什么
选择某个结果，以及每个结论由什么 evidence 支持。

如果每个调用方自行解释 `stale`、`suppressed`、`expired`、`archived`、
`compressed`、`superseded` 等状态，同一份数据可能产生不同答案；无数据、
证据不足或冲突也可能被错误地包装成确定事实。

## 目标

- 提供 versioned `EvidenceView`、`ClaimView`、`RelationView` 和
  `CurrentTruthView` 读取模型，让调用方消费同一套 truth 语义。
- 将 publication、validity、retention 与 visibility 分开表达，避免把存储、
  策略可见性或发布状态误当作事实真假。
- 对 scope、`as_of`、supersedes、evidence trust、冲突和证据不足采用可解释、
  确定性的选择规则。
- Phase A 先提供可重建的只读 projection；Phase B 再让 Context Bundle
  消费 projection；Phase C 仅在读取模型和质量证据稳定后评估 writer 收敛。
- 在所有阶段保留 canonical ref、evidence refs 和明确的
  `TruthSelectionReason`，使结果可以审计。

## 非目标

- 不立即替换现有数据库表，也不要求一次性迁移所有历史数据。
- 不引入新的图数据库或实时 multi-agent blackboard。
- 不让 LLM 任意裁决 truth，也不把模型自报 confidence 当作真实性分数。
- 不把所有 event 或 generated enrichment 自动提升为 canonical Claim。
- PR #939 不接入 Context Bundle，不交付 worktree/task selector，不修改
  canonical writer，也不声明 Phase B/C 完成。
- 本规格不把 archived、compressed 或 suppressed 简化为 false。

## Behavior Invariants

1. CT-001 projection 必须返回带版本的 `EvidenceView`、`ClaimView`、
   `RelationView` 和 `CurrentTruthView`；同一版本对同一可见输入和同一查询
   必须产生确定性相同的结果。
2. CT-002 lifecycle 必须分别表达 publication、validity、retention，并将
   visibility/policy suppression 独立表达；`Archived` 不等于无效，
   `Compressed` 不等于 false，`Suppressed` 不得自动改写 claim 的真假。
3. CT-003 查询必须先应用 scope 隔离。Phase A 至少支持 project 和 branch；
   project 不匹配的 claim 不得泄露，带 branch 的 claim 只在对应 branch
   中可见。worktree/task selector 属于 GH-933 后续阶段，不是 PR #939
   的完成项。
4. CT-004 指定 `as_of` 时，projection 只能使用该时点已存在且在有效时间窗内
   的 claim、relation 和 evidence。若底层数据没有足够历史信息恢复过去状态，
   必须暴露限制或返回 `Unknown`，不得根据当前值伪造历史 truth。
5. CT-005 在同一 subject 的候选 claim 中，时间上生效的显式
   `Supersedes` 必须优先于纯 recency，并在结果中记录被替代项和选择原因。
6. CT-006 verified evidence 必须优先于 model-generated 或 untrusted
   evidence；任何 stored confidence 都不得替代这条可解释的 trust 规则。
7. CT-007 当有效的 `Refutes` 或等价冲突无法安全裁决时，结果必须是
   `Contradicted`，并保留冲突双方及相关 relation；不得静默折叠成单一 truth。
8. CT-008 无匹配 claim、所有 claim 均无 current standing、证据不足或未知
   状态无法安全解释时，必须返回 abstention/`Unknown` 或明确空结果，不得
   发明 claim，也不得把失败伪装为正常空数据。
9. CT-009 每个非空 truth 或 conflict 结果必须携带 canonical ref、可用的
   evidence refs、相关 supporting/contradicting relations，以及机器可判定的
   `TruthSelectionReason`。
10. CT-010 generated enrichment 不得仅凭模型生成身份创建或覆盖 canonical
    Claim。Phase A 的读取结果必须排除不具备 claim standing 的 enrichment；
    writer 侧的统一防线属于 Phase C，PR #939 不宣称已完成。
11. CT-011 stale、expired、deleted、candidate、suppressed 和 archived 状态
    必须分别处理并有独立可观察结果；未知状态必须 fail closed，不能 panic、
    自动视为 current 或静默降级。
12. CT-012 archived evidence 可用于后续 historical explanation，但默认不得
    进入 current context。Phase A 可以保守排除 archived current truth；
    historical explanation 的完整消费契约属于后续阶段。
13. CT-013 Phase B 的 Context Bundle 只能消费 versioned projection 输出，
    并分别呈现 current truth、decision 和 conflict；切换期间必须保留可回滚
    的旧 context path，不能把 projection 失败降级成看似成功的缺失 context。
14. CT-014 Phase C 只有在 read model、冲突/abstention 行为和 benchmark
    稳定后才能评估 writer 收敛；收敛不得破坏已有 canonical ref、evidence
    provenance、scope 隔离或历史解释能力。
15. CT-015 projection 必须是只读、可重建且可版本化的。读取失败、schema
    不兼容或 evidence 解析失败必须返回可诊断错误或明确的 fail-closed 结果，
    不得修改 canonical 数据来完成查询。

## 验收标准

### PR #939：Phase A partial implementation

- [ ] 提供 versioned `EvidenceView`、`ClaimView`、`RelationView` 和
      `CurrentTruthView`，且 projection 对现有受支持对象保持只读。
- [ ] project、branch、`as_of`、supersedes、refutes、supports、evidence
      trust 和 selection reason 均有确定性行为验证。
- [ ] 每种现有受支持状态均有 lifecycle 映射验证；未知状态 fail closed。
- [ ] golden fixtures 覆盖 supersedes、双 evidence 支持、contradiction、
      branch isolation、`as_of` history、scope isolation 和 abstention。
- [ ] unresolved conflict 不被静默选择；无足够证据不产生 invented truth。
- [ ] PR body 保持 `Refs #933`，并明确 Phase B/C 未完成、GH-933 保持开放。

### GH-933：后续整体完成条件

- [ ] Phase B 让 Context Bundle 消费 projection，并验证 current truth、
      decisions、conflicts 与旧 path rollback。
- [ ] worktree/task selector 具备与 project/branch 一致的 scope isolation
      和不泄露验证。
- [ ] archived evidence 可参与 historical explanation，但默认不进入
      current context。
- [ ] Phase C 的 writer 收敛决定有 benchmark、迁移/兼容与 rollback 证据；
      若不收敛，也必须记录明确决定和边界。
- [ ] generated enrichment 在最终写入与读取契约中都不能创建或覆盖
      canonical Claim。
- [ ] 文档、调用方契约与发布说明同步后，才可声明 GH-933 整体完成。

## 边界情况

- 两个 claim 时间相同、evidence trust 相同，且没有可裁决的关系。
- 新 decision 在 `as_of` 之后 supersede 旧 decision。
- user-authored evidence、tool output 与 model-generated enrichment 冲突。
- branch A 与 branch B 对同一 subject 有不同 current truth。
- claim 被 suppressed 但仍可能在政策允许的历史审计中存在。
- archived evidence 仍有历史价值，但当前查询不应把它当作 active truth。
- 底层状态被原地改写或数据已删除，无法完整重建过去时点。
- 空 scope、无匹配数据、未知 stored status、损坏 evidence ref 或读取失败。

## 发布说明

PR #939 只发布 Phase A library-level read projection，不新增 CLI/HTTP 或
Context Bundle 用户界面，并继续以 `Refs #933` 关联开放 issue。Phase B/C
必须独立经过 spec approval、实现验证和发布说明；在这些阶段完成前，不得将
GH-933 或完整 CurrentTruth 能力标记为已交付。
