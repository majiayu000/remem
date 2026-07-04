# Tech Spec

## Linked Issue

GH-720

## Product Spec

`specs/GH720/product.md`

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| transcript 解析 | `src/memory/raw_transcript.rs` | 已支持 Claude Code（type:user/assistant）与 Codex rollout 两种格式的逐行解析 | Phase 1 原样复用，是"80% 已存在"的核心 |
| raw 摄入 | `src/memory/raw_archive.rs`（`drain_transcript`） | 按 transcript 路径逐行入 `raw_messages`，UNIQUE(project, role, content_hash) 幂等，`raw_ingest_failures` 记账 | 批量摄入的执行单元，per-file 调用即可 |
| 摄入触发点 | `src/summarize/summary_job/process.rs`（drain 调用处） | 仅 Stop hook 时对当前会话触发 | 保持不动；批量路径与其并发安全依赖同一 UNIQUE 约束 |
| raw 存储 | `src/migrations/v002_raw_messages.sql` | `raw_messages` + trigram FTS + `idx_raw_messages_project_created(project, created_at_epoch DESC)` | 时间窗口查询的索引已就绪 |
| raw 查询 | `src/memory/raw_archive.rs`（`RawSearchRequest`）、`src/mcp/server/raw_tools.rs`、CLI `remem raw` | query/project/branch/role/limit/offset，无时间界 | Phase 1 扩展点 |
| CLI 入口 | `src/cli/types.rs` | 子命令枚举；`Import` 目前仅支持备份导入 | 新增 `ingest-sessions` 子命令的挂载点 |
| job 基建 | migrations 中的 `jobs`、`extraction_tasks`、`ai_usage_events` | 后台任务、提取任务、LLM 用量记账已有表结构 | Phase 3 facet 提取的既有槽位 |
| 待移植源 | refine `packages/core/src/session/discovery.rs`、`apps/cli/src/ingest_sessions.rs` | 目录发现（两类根目录）+ 全局 mtime 游标增量 | Phase 1 的移植蓝本；游标粒度将从全局改为 per-file |
| refine 消费端 | refine facet 提取管线、insights、mirror、cognitive-portrait skill | 读 refine 自己的 `items/documents` | Phase 2/3 的迁移对象 |

## Proposed Design

### Phase 1a — `remem ingest-sessions`

新模块 `src/ingest/sessions.rs`：

1. **发现**：内置根 `~/.claude/projects`（layout: `<proj-slug>/*.jsonl`，排除 `subagents/`）与 `~/.codex/sessions`（layout: `YYYY/MM/DD/rollout-*.jsonl`）；`--root label=path` 可追加（含 remote-sessions 同步目录）。移植自 refine `discovery.rs`。
2. **游标**：新迁移建表 `ingest_cursors(file_path TEXT PRIMARY KEY, mtime_epoch INTEGER, size_bytes INTEGER, last_ingested_at INTEGER)`。粒度选 per-file 而非 refine 的全局 mtime：活跃会话文件会反复追加，全局游标会漏掉"旧文件新内容"。mtime 与 size 都未变 → 跳过；任一变化 → 重新 drain（幂等约束兜底重复行）。
3. **执行**：对每个命中文件调用现有 `drain_transcript(conn, path, session_id, project, branch, cwd)`；session_id 取文件名 stem，project 取目录 slug，来源根 label 随行写入（`raw_messages` 若无来源列则新迁移加 `source_root TEXT` 默认 `local`）。
4. **失败**：沿用 `raw_ingest_failures`；单文件失败 continue，退出码区分"全部成功/部分失败"。
5. **输出**：`--json` 汇总 `{scanned, skipped, ingested_messages, failed_files}`（product invariant 6）。
6. **半行截断**：drain 读到最后一行 JSON 解析失败且文件 mtime 在最近 60s 内 → 视为活跃追加，记为 partial 而非 failure，游标不前进到该文件（下轮重读尾部）。

### Phase 1b — 时间窗口查询

1. `RawSearchRequest` 增加 `since_epoch: Option<i64>` / `until_epoch: Option<i64>`，SQL 走 `idx_raw_messages_project_created`；缺省行为不变（invariant 7）。
2. 新查询 `list_sessions(window, project, sample_n)`：对 `raw_messages` 按 `(source_root, project, session_id)` 分组，返回窗口内 min/max epoch、消息计数，及每会话按 epoch 升序前 N 条 role=user 的消息文本截断。一次 SQL（窗口聚合）+ 每会话一次采样查询，会话数有限（窗口内典型 <100）可接受。
3. CLI：`remem raw --since --until`；`remem raw sessions --since --until --project --sample N --json`。MCP：`raw_tools` 增加对应工具，输出字段与 CLI JSON 一致（invariant 10）。HTTP 面本阶段不加（无消费方），tech 债记录在案。

### Phase 2 — refine 切换输入源

refine 侧改动（在 refine 仓库执行，本 spec 只定接口契约）：

- 输入源从自扫目录改为 `remem raw sessions --json` + `remem raw --json` 子进程调用（选 CLI JSON 而非 HTTP：无守护进程依赖、无端口管理；HTTP 留作后续可选）。
- 对账：切换前后各跑一次相同窗口的 facet 提取，diff 数量与维度分布，差异归因写入 GH-720 评论（invariant 12）。
- refine 的 discovery/ingest 代码路径以 feature flag 停用，保留一个 release 周期后删除。

### Phase 3 — facet 内化

- 新迁移：`facets(id INTEGER PK, session_id TEXT, project TEXT, source_root TEXT, dimension TEXT, content TEXT, source_epoch_start INTEGER, source_epoch_end INTEGER, model TEXT, created_at_epoch INTEGER)` + `(dimension, created_at_epoch)` 索引。独立表，不复用 `observations`（决策记录：12 维结构压进 observations 会破坏其"chronological facts"语义）。
- 提取作为 `jobs`/`extraction_tasks` 的新任务类型，输入为窗口内未提取的会话；LLM 调用记 `ai_usage_events`。
- 回填上限：配置项 `facet_backfill_days`（默认 90），超出窗口的历史不自动提取（invariant 14）。
- 消费方迁移与 refine 归档在本阶段收尾，各开独立 issue。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 幂等 | `ingest/sessions.rs` + UNIQUE 约束 | 集成测试：fixture 目录连跑两次，第二次 ingested_messages == 0 |
| P2 增量 | `ingest_cursors` | 单测：mtime/size 未变跳过；改 size 后重摄 |
| P3 多根 + 来源标识 | discovery + `source_root` 列 | 单测：两个根下同名 project 不串扰 |
| P4 单文件失败隔离 | drain 循环 + `raw_ingest_failures` | 集成测试：坏文件混入，其余文件正常入库，退出码=部分失败 |
| P5 与 hook 并发去重 | UNIQUE 约束 | 集成测试：同一 transcript 先 hook drain 再批量，无重复行 |
| P7 窗口过滤向后兼容 | `RawSearchRequest` | 单测：无窗口参数时结果与现版本快照一致 |
| P8/P9 会话列表 + 采样 | `list_sessions` | 单测 + 人工：与 recap 脚本同窗口对账（会话数一致） |
| P11/P12 refine 切换 | refine 仓库 | 对账报告归档于 GH-720 |
| P13 facets 独立表 | Phase 3 迁移 | migration 测试 + schema review |
| P14 回填成本上限 | `facet_backfill_days` | 单测：超窗会话不产生 extraction task |

## Data Flow

```
~/.claude/projects/**.jsonl ─┐
~/.codex/sessions/**.jsonl  ─┼─ discovery ─ ingest_cursors(增量) ─ drain_transcript ─ raw_messages(+source_root)
remote-sessions/<host>/**   ─┘                                          │
Stop hook（现有，不变）──────────────────────────────────────────────────┘
raw_messages ─ RawSearchRequest(+since/until) ─ CLI / MCP ─→ recap、refine(Phase 2)、其他消费方
raw_messages ─ extraction job(Phase 3) ─ facets ─→ cognitive-portrait 类消费方
```

外部调用：仅 Phase 3 的 facet 提取消费 LLM API（记账 `ai_usage_events`，回填有硬上限）。

## Alternatives Considered

- **refine 保留独立存储、对外提供 API**：拒绝——双 schema 摄入不消除，refine 内部 schema 被抬成 serving 契约，产品叙事分裂（讨论记录见 GH-720 正文）
- **refine `items/documents` → remem 的库级迁移**：拒绝——原始 jsonl 尚在，重摄入比 schema 映射脚本干净且幂等安全；代价（facet 历史需重提取）由 90 天上限控制
- **facet 压进 `observations`**：拒绝——扭曲现有表语义；独立 `facets` 表的代价只是一张表
- **全局 mtime 游标（refine 现状）**：拒绝——活跃会话文件持续追加，全局游标漏"旧文件新内容"；per-file 游标多一张小表，正确性换值得
- **Phase 1b 直接上 HTTP 面**：推迟——当前无 HTTP 消费方，先 CLI/MCP 两面，避免 U-26 式"声明未接线"

## Risks

- Security: transcript 含敏感内容，批量摄入扩大了 SQLCipher 库内的敏感面；不新增网络出口，风险与现有 raw archive 同级。remote-sessions 根引入他机数据，`source_root` 必须如实标注来源
- Compatibility: `raw_messages` 加 `source_root` 列需迁移，默认值 `local` 保证旧行为；查询面新增参数全部 Option，无破坏性变更
- Performance: 首次全量回填（本机 + starlight ≈ 数百文件 / 百 MB 级）流式解析，预估分钟级；游标使后续运行秒级。`list_sessions` 聚合在窗口索引上，量级安全
- Maintenance: Phase 2 期间两仓库存在接口耦合（CLI JSON 契约），契约字段变更须在两仓库同步——对账报告是变更哨兵

## Test Plan

- [ ] Unit tests: discovery 根解析/排除规则、游标跳过与失效、窗口参数 SQL、半行截断判定
- [ ] Integration tests: fixture 目录幂等双跑、坏文件隔离、hook+批量并发去重、`list_sessions` 采样正确性
- [ ] Manual verification: 真实数据全量回填后，`remem raw sessions --since <7d>` 与 recap 脚本同窗口输出对账（会话数、每会话消息数一致）

## Rollback Plan

- Phase 1 全部增量：删除 `ingest-sessions` 子命令调用即回到现状；`ingest_cursors`/`source_root` 迁移保留不影响旧路径
- Phase 2：refine 侧 feature flag 切回自扫路径（保留一个 release 周期）
- Phase 3：facets 表独立，停用 extraction job 类型即冻结，无级联影响
