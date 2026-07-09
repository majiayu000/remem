# GH684 Product Spec: Legacy observation dual-path retire-vs-freeze

Issue: https://github.com/majiayu000/remem/issues/684
Route: write_spec
Locale: zh-CN
Status: Draft, needs human approval before implementation
Related:
- 权威契约：`docs/specs/legacy-observation-retirement/PRODUCT.md` + `TECH.md`
- SpecRail packet：`specs/GH684/product.md`、`tech.md`、`tasks.md`
- 反重写收敛契约：`docs/specs/current-memory-contracts/`（Refs #381/#383/#384）

## 1. 背景

remem 里有两代存储机制同时在跑：

- 旧路径（pre-v006）：`pending_observations` 队列（`src/db/pending/`）、
  `observations`（+`observations_fts`）、`session_summaries` 的写入/finalize
  链路（`src/db/summarize/session/`），外加旧迁移助手
  （`migrate_legacy_pending`、截断 shim）。
- 当前路径（capture ledger）：`captured_events` → `extraction_tasks` →
  `memory_candidates` → `memories`。

2026-07-02 的静态清点 + dogfood 数据核对（schema v53、42k memories、8.3k
sessions）把"legacy"这个词收窄成三种不同性质，而不是一整条并行管线：

- `pending_observations` 是**死队列**：默认运行路径上没有写入者，dogfood
  库每种状态都是 0 行；claim/lease 机制随二进制发布但无生产调用方。
- `observations`（+`observations_fts`）其实是**当前抽取管线的活跃中间态**，
  只是 MCP/文档一度把它标注成"legacy observations"（措辞错误，已由
  GH684-T8 修复）。
- `session_summaries` 表**载荷关键**，但被**双写**：当前的
  `SessionRollup` 任务和旧的 `JobType::Summary` 作业链都从同一个 Stop hook
  无条件触发。旧链在 dogfood 库上还堆了 2479 个失败作业和 24019 次无归属
  的 AI 调用。

## 2. 问题

双路径给每个检索/排序/新鲜度特性都加了"双读税"，并放大边界情况（哪个源
优先、哪个 FTS 索引权威）；审计反复标记 dual-schema 复合失败模式；新贡献者
要改一个行为得先学两条管线。旧路径还在 user-facing 表面泄漏：MCP
`get_observations` 曾把 `source='observation'` 描述成"legacy observations"
（`src/mcp/server/context_tools.rs`），timeline/context 查询读
`session_summaries`，doctor 追踪 `pending_observations` 活性。

## 3. 目标

- **P1 每个 legacy 表面一个明确、已记录的处置决策**：retire（迁移后 drop）
  vs freeze（只读、打标、带移除日期）。决策落在 spec 里（见 TECH.md 决策
  矩阵）。
- **P2 完整的 writer/reader 清单**：带 file:line，决策基于事实而非记忆。
- **P3 零数据丢失**：有独特价值的行在任何 drop 前先迁移，且走弃用窗口。
- **P4 legacy 状态可观测**：doctor 报告 legacy 行数、last-write、以及冻结
  表面是否仍在被写入。
- **P5 表面清理**：一旦冻结，MCP/CLI/REST 默认不再宣传 legacy 源；
  `source='observation'` 在移除前作为显式 opt-in 审计路径存在。

## 4. Non-Goals

- **N1** 不做第二次大重写（anti-rewrite 收敛）；本 spec 只把表面收敛到已经
  胜出的 capture-ledger 管线。
- **N2** 不改动当前 capture ledger 路径本身的行为。
- **N3** 不在普通迁移里静默 drop 表；每个 drop 自带迁移、release note、
  doctor 预检。
- **N4** timeline/context 特性不丢能力；只有在替换源被证明等价后才切数据源。
- **N5** 不移除 `observations`、`observations_fts`、`session_summaries`——
  核对后处置为 current/load-bearing。

## 5. 验收标准

- [ ] TECH.md 的 writer/reader 清单随每个生产写者/读者保持最新。
- [ ] `finalize_summarize` 与 `persist_session_rollup` 的字段级等价 fixture
      在 Summary 作业退役前完成对比。
- [ ] `pending_observations` 空态在主 dogfood 之外的真实库上也被确认，或残留
      行被显式迁移。
- [ ] 升级时刻在途 `JobType::Summary` 作业的处理（排空/拒绝/转换）已决策并
      测试。
- [x] MCP/文档措辞停止把活跃 `observations` 叫 legacy（GH684-T8）。
- [ ] doctor 报告 legacy 行数，并在冻结表面收到写入时报错。
- [ ] 每个 drop 走弃用窗口 + guarded 迁移（预检拒绝残留价值行）。
