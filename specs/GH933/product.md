# Product Spec：CurrentTruth 统一读取契约

## Linked Issue

GH-933

PR #939 已于 2026-07-26 合并（merge commit `0ed42e3d`），仅交付 Phase A
只读 projection baseline，并以 `Refs #933` 保持 GH-933 开放。维护者已授权
为剩余 Phase A hardening 使用新的 follow-up PR；该授权不是对本次修订后
spec packet 的 `spec_approval`，也不表示 Phase B 或 Phase C 已完成。

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
- Phase A baseline 先提供可重建的只读 projection；下一份 Phase A hardening
  follow-up 补齐 scope/relation 隔离、可诊断 provenance、只读证明与 bounded
  relation lookup；Phase B 再让 Context Bundle 消费 projection；Phase C 仅在
  读取模型和质量证据稳定后评估 writer 收敛。
- 在所有阶段保留 canonical ref、evidence refs 和明确的
  `TruthSelectionReason`，使结果可以审计。

## 非目标

- 不立即替换现有数据库表，也不要求一次性迁移所有历史数据。
- 不引入新的图数据库或实时 multi-agent blackboard。
- 不让 LLM 任意裁决 truth，也不把模型自报 confidence 当作真实性分数。
- 不把所有 event 或 generated enrichment 自动提升为 canonical Claim。
- Phase A hardening follow-up 不接入 Context Bundle，不交付 worktree/task
  selector，不修改 canonical writer/schema，也不声明 Phase B/C 完成。
- 本规格不把 archived、compressed 或 suppressed 简化为 false。

## Behavior Invariants

1. CT-001 projection 必须返回带版本的 `EvidenceView`、`ClaimView`、
   `RelationView` 和 `CurrentTruthView`；同一版本对同一可见输入和同一查询
   必须产生确定性相同的结果。Phase A hardening 的 v2 subject identity 必须
   显式区分 source、kind 与 key；memory subject 至少由
   `(ClaimSource::Memory, memory_type, topic_key-or-singleton)` 构成，不能让不同
   `memory_type` 仅因 `topic_key` 相同而竞争。
2. CT-002 lifecycle 必须分别表达 publication、validity、retention，并将
   visibility/policy suppression 独立表达；`Archived` 不等于无效，
   `Compressed` 不等于 false，`Suppressed` 不得自动改写 claim 的真假。
3. CT-003 查询必须先应用 scope 隔离。Phase A 至少支持 project 和 branch；
   project 不匹配的 claim 不得泄露。`branch=Some(B)` 明确表示 branch-scoped
   查询，只能看到 branch-neutral 与 exact `B` rows；`branch=None` 明确表示
   branch-agnostic 查询，可以看到该 project 的全部 branch rows，而不是
   branch-neutral-only 的隐式默认。relation 的两个 endpoint 也必须同时属于
   该查询的 scoped claim set。会改变 winner 的 `Supersedes`/`Refutes` 两端还
   必须属于同一 typed subject；scope 外 relation 不得改变 scope 内结果。
   `Supports`/`DerivedFrom` 等 provenance-only relation 可以连接同一 scope
   内的不同 typed subject，但只能作为 winner 的 provenance 输出，不能参与
   survivor、trust 或 recency 决策。worktree/task selector 属于 GH-933
   后续阶段，不是 Phase A hardening 的完成项。
4. CT-004 指定 `as_of` 时，projection 只能使用该时点已存在且在有效时间窗内
   的 claim、relation 和 evidence。captured evidence 必须同时满足 source time
   `COALESCE(reference_time_epoch, created_at_epoch) <= as_of` 与 remem knowledge
   time `inserted_at_epoch <= as_of`；source event 虽早、但在 `as_of` 后才被
   ingest 的 late evidence 不得回溯改变历史 winner。若底层数据没有足够历史
   信息恢复过去状态，必须暴露限制或返回 `Unknown`，不得根据当前值伪造历史
   truth。
5. CT-005 在同一 subject 的候选 claim 中，时间上生效的显式
   `Supersedes` 必须优先于纯 recency，并在结果中记录被替代项和选择原因。
6. CT-006 verified evidence 必须优先于 model-generated 或 untrusted
   evidence；任何 stored confidence 都不得替代这条可解释的 trust 规则。
   memory 引用的 captured event 必须通过 canonical project identity 证明属于
   与该 memory 相同的 exact project；foreign-project evidence 不得被降级后
   继续，也不得抬高本项目 claim。
7. CT-007 当有效的 `Refutes` 或等价冲突无法安全裁决时，结果必须是
   `Contradicted`，并保留冲突双方及相关 relation；不得静默折叠成单一 truth。
8. CT-008 无匹配 claim、所有 claim 均无 current standing、证据不足或未知
   状态无法安全解释时，必须返回 abstention/`Unknown` 或明确空结果，不得
   发明 claim，也不得把失败伪装为正常空数据。
9. CT-009 每个非空 truth 或 conflict 结果必须携带 canonical ref、可用的
   evidence refs、相关 supporting/contradicting relations，以及机器可判定的
   `TruthSelectionReason`。同 scope 的 cross-subject provenance-only relation
   在连接 winner 时必须保留；“不同 subject 不参与 winner 决策”不得被错误实现
   成“丢弃所有 cross-subject provenance”。
10. CT-010 generated enrichment 不得仅凭模型生成身份创建或覆盖 canonical
    Claim。Phase A 的读取结果必须排除不具备 claim standing 的 enrichment；
    writer 侧的统一防线属于 Phase C，Phase A hardening 不宣称已完成。
11. CT-011 stale、expired、deleted、candidate、suppressed 和 archived 状态
    必须分别处理并有独立可观察结果；未知状态必须 fail closed，不能 panic、
    自动视为 current 或静默降级。Phase A 的实际 claim sources（memory 与
    user-context claim）遇到 unknown raw status 必须返回包含 table、canonical
    ref 与 raw status 的 contextual error，不能只映射为不可见 `Unknown` 而
    没有 diagnostic。
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

### 已合并 baseline：PR #939

PR #939 是已合并的 Phase A baseline evidence：提供 versioned DTO、lifecycle
mapping、read adapter、deterministic resolution 和 18 个 truth tests。它不是
本轮 fresh verification，也不覆盖以下 hardening 验收项。

### 下一份 Phase A hardening follow-up

- [ ] relation 两端都必须属于本次 query 的 scoped claim set。
      `Supersedes`/`Refutes` 只有同一 typed subject 才能改变 resolution；
      cross-project、explicit-branch-scope 外与 cross-subject decision relation
      不影响 winner。连接 winner 的 scoped cross-subject
      `Supports`/`DerivedFrom` 作为 provenance 输出保留但不参与选择，并有正反
      fixtures。`branch=None` 的 branch-agnostic 全分支语义和
      `branch=Some(B)` 的 neutral-plus-exact 语义都有独立 regression。
- [ ] `memory_edges` 与 trusted memory-to-memory `graph_edges` lookup 在 SQL
      或等价 bounded lookup 中受 scoped IDs 约束，不扫描无关项目全表；提供
      `EXPLAIN QUERY PLAN` 与 representative p50/p95/row-bound evidence。
- [ ] `ClaimRelationKind::Supports` 有真实 adapter/output fixture。
- [ ] 每个 memory evidence ref 通过 `captured_events.project_id` join canonical
      `projects.project_path`，必须与 `memory.project` exact match；无法解析
      project identity 或 foreign-project ref 都返回含 memory/event/project
      context 的错误，不能静默降级 trust。
- [ ] explicit `as_of` 查询只使用 source time 与 knowledge time 都不晚于
      `as_of` 的 captured evidence；覆盖 source-before/knowledge-after 的 late
      ingestion、两种时间的 before/equal/after boundary 与 historical winner。
      `as_of=None` 仍共享一次 reference epoch。
- [ ] malformed `evidence_event_ids`、malformed `source_refs_json`，以及
      syntactically valid 但指向不存在 `captured_events` row 的 dangling event
      ref 均 fail closed，并返回包含 claim/ref context 的可诊断错误；不得静默
      形成较低 trust 的“正常成功”。
- [ ] SQLite authorizer、`total_changes` 或等价 regression 证明 projection
      SELECT-only。
- [ ] memory subject identity 与 canonical writer 对齐为
      `(source, memory_type, topic_key-or-singleton)`；相同 topic、不同
      `memory_type` 绝不竞争或 supersede。该合法输入变化触发上一轮既定 version
      boundary：hardening 输出必须为 `projection_version=2`，DTO/module paths、
      v1-to-v2 intentional golden diff 与 staged/unreleased compatibility note
      都在 exact packet 中批准；不得继续标 v1 或无审核改写 golden。
- [ ] unknown memory/user-claim raw status 返回包含 table/canonical-ref/raw-value
      的 contextual error，并覆盖每个实际 claim-source adapter；已知 status
      的 total mapping 不回归。
- [ ] final rebase、current-contract/version metadata 更新全部完成后，在最终
      candidate head 重跑完整 bounded-query/performance/golden gate；evidence
      绑定 exact SHA 与 relevant migration/index/truth/dependency fingerprints，
      其后任何 commit/rebase 都使 evidence 失效并要求重跑。
- [ ] follow-up PR 使用 `Refs #933`，只修改批准的 Phase A hardening、当前
      `docs/specs/GH933/{PRODUCT,TECH}.md` 契约与必要的 release metadata；
      current contract 必须移除已被 hardening 纠正的过时完成声明，不接入
      Context Bundle/writers/schema，不关闭 GH-933。

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
- branch A 与 branch B 对同一 subject 有不同 current truth；显式 branch 查询
  只消费 neutral+exact rows，而 `branch=None` 明确形成 branch-agnostic 竞争。
- claim 在 `as_of` 前存在，但其 verified captured-event evidence 在 `as_of`
  后才被 remem ingest，source time 可能仍早于 `as_of`；或引用的 ledger event
  已不存在/属于另一个 project。
- 两个不同 `memory_type` 使用相同 `topic_key`；它们必须形成不同 typed
  subjects。
- scoped `Supports`/`DerivedFrom` 连接 winner 与另一个 typed subject；该
  provenance 保留，但不能改变 winner。
- memory 或 user-context claim 存有未识别 raw status。
- claim 被 suppressed 但仍可能在政策允许的历史审计中存在。
- archived evidence 仍有历史价值，但当前查询不应把它当作 active truth。
- 底层状态被原地改写或数据已删除，无法完整重建过去时点。
- 空 scope、无匹配数据、未知 stored status、损坏 evidence ref 或读取失败。

## 发布说明

PR #939 已合并 Phase A library-level read projection baseline，但对应 source
version/release manifest 仍是 staged/unreleased，不能声称已向 installed-binary
用户发布；它也没有新增 CLI/HTTP 或 Context Bundle 用户界面。下一份 Phase A
hardening follow-up 仍以 `Refs #933` 关联开放 issue，并按实现时 `main` 的版本
顺序做一次必要的 patch version 同步；不在本规格中预占具体版本号。只有实际
release artifact 发布并经验证后才能使用 “released/published” 表述。Phase B/C
必须独立经过 spec approval、实现验证和发布说明；在这些阶段完成前，不得将
GH-933 或完整 CurrentTruth 能力标记为已交付。
