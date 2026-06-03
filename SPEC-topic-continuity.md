# SPEC: Topic Continuity（话题连续性）

- 状态：Phase 0 Gate **已通过**（2026-06-02，见 §0），待评审
- 作者：remem 维护者 + Claude
- 日期：2026-06-02
- 关联缺口：检索/记忆缺口 B（话题连续性）—— 见会话分析
- 灵感来源：Membox（Topic Loom / Trace Weaver，按转述理解，**未独立核实论文**）

---

## 0. Phase 0 Gate 验证结果（2026-06-02，已通过）

用加密前备份 `remem.db.bak` 的真实 session 1707（43 事件、跨度 17h、含一处 995min 大 gap）跑分段 prompt（MiMo `mimo-v2.5`，prompt 22127 / completion 1791 tokens，一次调用，exit 0）。

**结论：PASS。** 切分质量达标：
- LLM 切出 6 个语义清晰的话题段（corpus 抓取启动 / 反爬调研 / kexue 实现 / yam-baoyu 实现 / 质量验证 / 训练启动），topic_key 为稳定 kebab-case。
- **A 辅信号生效的直接证据**：995min 大 gap 被正确用作边界——gap 前事件（≤3466）归前 4 段，gap 后（≥4126）归后 2 段。
- 整体 summary 事实准确（1158 篇文章、MAX_LEN=8192 等），无明显幻觉。

**关键发现（已据此修正 §4.1）**：并行/交错任务下，**话题段的事件区间会重叠/嵌套**（如"反爬调研"段 3056-3466 与"kexue 实现"段 3057-3331 嵌套，因多个 background agent 并行）。因此：
- segment **不能**假设为"连续不重叠区间"。
- `evidence_event_ids`（离散集合）是 segment↔事件的**权威关联**；`covered_from/to_event_id` 仅作 min/max 派生（排序/范围查询用）。
- 幂等键不能用 `(from,to)`。

---

## 1. 背景与现状（事实，带代码证据）

remem 当前把"提取"与"话题"完全解耦，导致记忆是离散碎片，无法按话题串起时间线：

| 现状 | 证据 |
|---|---|
| 提取边界 = 事件 ID 范围（`cursor..high_watermark`），与话题无关 | `src/session_rollup.rs:80-145`（`load_rollup_range`） |
| `session_rollup` 把整段事件一次性汇总成**一条纯文本** summary | `src/session_rollup.rs:74-77, 169-212`（`persist_session_rollup`） |
| `session_summaries` 只存 `summary_text` + `covered_from/to_event_id`，无内部话题切分 | `src/session_rollup.rs:188-209` |
| memory 是离散条目，`topic_key` 仅用于去重/演化，条目间无顺序/因果 | `memory_candidates.topic_key` `src/migrations/v006_capture_pipeline.sql:102` |
| 检索能命中单条，但不能"把同话题的若干记忆按时间顺序串回" | 4-channel RRF + 固定 2-hop entity 共现（README "Search Architecture"） |

**已有的半个地基**：`topic_key` 概念已存在于 `memory_candidates` / `memories`，只是它是对**单条 candidate** 生成的，不是对"一段连续对话"生成的，也没有按时间线组织的结构。本 SPEC 复用 `topic_key`，不另造概念。

---

## 2. 目标 / 非目标

### 目标
1. **Topic Loom（按话题分段存储）**：在 `session_rollup` 阶段，把事件范围切成若干"话题段"（topic segment），每段是一个连贯单元，带 `topic_key` + 标题 + 小结 + 事件范围 + 证据链。
2. **Trace Weaver（按话题串时间线）**：跨段 / 跨 session 的同 `topic_key` 段，能按时间排序聚合成一条 trace（话题时间线）。
3. **检索/注入衔接**：命中某条记忆/段时，能带出其所属 trace 的前后段（标题 + 关键结论）。

### 非目标（本 SPEC 明确不做）
- ❌ 不引入向量 / embedding 检索（`src/retrieval/vector.rs` 仍是 TODO，那是独立大工程，见缺口 A）。
- ❌ 不改 `memories` 作为"最终晋升产物"的地位——`topic_segments` 是**中间层**，不是新的真实来源（避免 U-12 / RS-12 双轨）。
- ❌ 不依赖 Claude 主动调用任何工具（CLAUDE.md 错误 2：自动化捕获是主力）。
- ❌ 不新增 LLM 调用次数（复用 `SessionRollup` 那一次调用）。

---

## 3. 边界判定策略：B 为主 + A 为辅（已决策）

- **B（主）：LLM 语义切分**。`SessionRollup` 的 LLM 调用看到完整有序事件流，直接输出话题分段。质量最高，零额外成本。
- **A（辅）：启发式信号作为 prompt 提示**，不做硬切分。在构造 prompt 时为每个 event 计算并标注：
  - `gap_before`：与上一个 event 的时间间隔（秒），由 `captured_events.created_at_epoch` 算出（`src/session_rollup.rs:29` 已加载该字段）。
  - `files_touched`：该 event 涉及的文件（来自 tool 输入/输出，若可得）。
  - `turn_id`：`captured_events.turn_id`（`v006:61` 已有）变化。
  这些作为 `<event ... gap_before="1800">` 形式的提示标注，由 LLM 自行参考。**最终边界由 LLM 决定。**
- **C（嵌入滑窗）：本 SPEC 不采用**，理由见非目标。

---

## 4. 数据模型

### 4.1 新表 `topic_segments`（Phase 1）

中间层，介于 `observations`/`session_summaries` 与 `memories` 之间。新迁移 `src/migrations/v022_topic_segments.sql`：

```sql
CREATE TABLE IF NOT EXISTS topic_segments (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER NOT NULL REFERENCES sessions(id),
    project TEXT NOT NULL,                  -- 与现有表保持冗余 project 文本列一致
    topic_key TEXT NOT NULL,                -- 复用现有 topic_key 命名规则
    title TEXT NOT NULL,
    summary TEXT NOT NULL,                  -- 该段小结
    status TEXT NOT NULL,                   -- open | resolved | superseded
    segment_index INTEGER NOT NULL,         -- LLM 输出顺序(非区间序)；段区间可重叠/嵌套
    covered_from_event_id INTEGER NOT NULL, -- 指回 captured_events，证据链不丢
    covered_to_event_id INTEGER NOT NULL,
    evidence_event_ids TEXT NOT NULL,       -- JSON 数组，与 observations 一致
    files TEXT,                             -- JSON 数组，可空
    confidence REAL NOT NULL DEFAULT 0.75,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

-- trace 聚合主路径：同 project 同 topic_key 按时间排序
CREATE INDEX IF NOT EXISTS idx_topic_segments_trace
    ON topic_segments(project_id, topic_key, covered_from_event_id);
-- 幂等去重 + 按 session 回放
CREATE INDEX IF NOT EXISTS idx_topic_segments_session
    ON topic_segments(session_row_id, segment_index);
```

**段允许重叠/嵌套**（Phase 0 实测：并行任务下区间会嵌套，见 §0）：`evidence_event_ids` 是权威关联，`covered_from/to_event_id` 仅为派生 min/max。**幂等键 = `(session_row_id, topic_key)`**（同次 rollup 内同 topic_key 合并），**不用** `(from,to)`。

### 4.2 Trace（Phase 2）—— 先用动态聚合，不建表

一条 trace = `SELECT * FROM topic_segments WHERE project_id=? AND topic_key=? ORDER BY covered_from_event_id ASC`。

物化表 `topic_traces` 推迟到 trace 数量大、需要预算控制时再做（配合 `dream` 周期重算）。本 SPEC 不实现。

---

## 5. LLM 契约（复用现有 XML 解析风格）

`SessionRollup` 的一次 LLM 调用，输出从"一段纯文本"升级为"summary + segments"。**`summary_text` 继续保留并落库**，保证现有 context 消费方（session section）零破坏；segments 是纯增量。

### 5.1 System prompt 增量

在现有 `SESSION_ROLLUP_SYSTEM`（`src/session_rollup.rs:9-12`）基础上追加：

> 同时把事件按话题切成若干连贯段落。一个话题段 = 围绕同一目标/问题/文件的连续讨论。
> 参考每个事件上的 `gap_before` / `turn_id` / `files_touched` 提示，但由你判断真实话题边界。
> 每段给出 topic_key（稳定 kebab-case，同一话题跨会话应一致）、title、summary、起止 event id、status。

### 5.2 输出格式

```xml
<summary>整体会话小结（向后兼容，写入 session_summaries.summary_text）</summary>
<segments>
  <segment topic_key="fts5-trigram-tokenizer" status="resolved">
    <title>切换 FTS5 到 trigram tokenizer</title>
    <summary>决定用 trigram 以支持 CJK；验证了 bm25 权重。</summary>
    <from_event_id>123</from_event_id>
    <to_event_id>131</to_event_id>
    <files>src/retrieval/search/common.rs</files>
  </segment>
</segments>
```

### 5.3 解析与校验（U-29 不静默降级）
- `from/to_event_id` 必须落在本次 `RollupRange` 内（`src/session_rollup.rs:138-139`），否则丢弃该段并 `log::warn`（段级失败不影响 summary 落库）。
- `topic_key` 为空 → 该段丢弃并告警。
- 整体 `<summary>` 解析失败 → 整个 task 失败重试（与现有 rollup 失败语义一致），**不**写半截数据。
- segments 全为空但 summary 正常 → 允许（退化为旧行为），记 `info`。

---

## 6. 代码落点

> `src/session_rollup.rs` 当前 433 行（生产 ~250 + 测试 ~180）。加 segment 逻辑会破 200 行红线（U-16），**必须拆模块**。

| 文件 | 动作 |
|---|---|
| `src/migrations/v022_topic_segments.sql` | 新建表（§4.1） |
| `src/session_rollup.rs` → 拆为 `src/session_rollup/mod.rs` | 拆分；`mod.rs` 保留 task 入口与 range 加载 |
| `src/session_rollup/prompt.rs` | 构造带 `gap_before/turn_id/files` 提示标注的 prompt（§3 启发式 A） |
| `src/session_rollup/parse.rs` | 解析 `<summary>` + `<segments>`（§5.3 校验） |
| `src/session_rollup/persist.rs` | 写 `session_summaries`（不变）+ 写 `topic_segments`（新，幂等） |
| `src/db/...`（新增读写函数） | `insert_topic_segment`、`topic_segments_exist`、`load_trace_by_topic_key` |
| **Phase 3** `src/retrieval/...` | trace 召回：命中 memory 后按其 topic_key 拉同话题段时间线 |
| **Phase 3** `src/context/sections/...` | 新增 "Topic timeline" section，受 12K 预算门控（`src/context/injection_gate.rs`） |

---

## 7. 分阶段实施（每阶段独立可验证）

- **Phase 0 — Gate（切分可行性验证，开工前必过）**：见 §8.1。LLM 切分准确率不达标则停，重新评估 prompt 或退回方案 A。
- **Phase 1 — Topic Loom**：迁移 + rollup 拆模块 + 输出 segments + 落 `topic_segments`。**本阶段独立有价值**（更连贯的 session 回顾），可单独发布。
- **Phase 2 — Trace 动态聚合**：`load_trace_by_topic_key`，CLI/MCP 暴露 `remem trace <topic_key>` 只读查询。
- **Phase 3 — 检索/注入衔接**：trace 召回 + context timeline section。这一步才真正抬升检索指标。

---

## 8. 验证项（W-03 / W-16：每步都要本会话内可复现的命令证据）

### 8.1 Phase 0 Gate：切分质量
- 取真实 `~/.remem` 中某 session 的 `captured_events`，构造 §5 prompt，跑一次 LLM。
- 人工评估：切分边界是否符合直觉（目标：≥80% 段边界合理）。
- 不达标不进入 Phase 1。

### 8.2 单元测试（Phase 1）
- `segments` 解析正确；`from/to` 越界被丢弃且告警；**允许段区间重叠/嵌套**（不得断言不重叠）；以 `evidence_event_ids` 为权威关联。
- 空 range 不写段（对齐 `session_rollup_empty_range_writes_no_summary`，`src/session_rollup.rs:305`）。
- 幂等：同 covered range 重跑不重复写段。
- `summary_text` 仍正常落库（向后兼容）。

### 8.3 集成 / 回归
- `cargo check` && `cargo test`（提交前必过）。
- LoCoMo before/after：重点看 **Temporal / Multi-hop** 两列（当前 v2 基线 Temporal 40.5% / Multi-hop 61.3%，README:210）。话题连续性应主要抬升这两项。
- `eval/local`：`python3 eval/local/run_local_eval.py --n 20`。

---

## 9. 风险 / 取舍

| 风险 | 缓解 |
|---|---|
| LLM 切分边界不稳定（同段两次切法不同） | `topic_key` 归一化复用现有规则对齐跨 session 段；Phase 0 Gate 先验证 |
| prompt 变长增加单次 token | 仍是一次调用，net 成本远低于新增 task_kind；rollup 本就喂全量事件 |
| `topic_segments` 与 `memories` 职责混淆 | 明确 segment 为中间层，memory 仍是晋升终点（§2 非目标） |
| 破坏现有 summary 消费方 | `summary_text` 输出与落库不变，segments 纯增量 |
| 单文件超 200 行 | 拆 `src/session_rollup/` 模块目录（§6） |

---

## 10. 回滚

- Phase 1 失败：停止写 `topic_segments`（feature 由 worker 分支控制），保留迁移表（空表无害）；rollup 回退为只写 `summary_text`。
- 迁移单向：`v022` 仅 `CREATE TABLE`，不改既有表，回滚无数据风险。

---

## 11. 开工前待确认

1. Phase 0 Gate 是否现在执行？需要 `.env` 的 `OPENAI_API_KEY` + 读取真实 `~/.remem` 数据 + 一次真实 LLM 调用。
2. `topic_key` 跨 session 一致性：是否复用 `memory_candidate` 现有的 topic_key 生成规则？（建议是）
3. Phase 1 是否作为独立 PR 先发（不含检索衔接）？

---

## 12. 进度（截至 2026-06-03）

PR #294（branch `feat/topic-continuity-segments`），全量 694 lib 测试绿。
- [x] Phase 1.1 v022 迁移（`1cc7111`）
- [x] Phase 1.2 db 层 insert + 幂等（`0137f96`）
- [x] Phase 1.3 rollup 接入（`de84d0e`，闭合 U-26）
- [x] Phase 2 读层 `load_trace_by_topic_key`（`0bc2c70`）
- [x] Phase 2 暴露 `remem trace <topic_key>`（`2d1add4`）
- [ ] Phase 3 检索/注入衔接 + LoCoMo

---

## 13. Phase 3 落点（下一会话直接执行）

在 worktree `remem-topic-continuity` / 分支 `feat/topic-continuity-segments` 继续。

1. **新建 `src/context/sections/topic_timeline.rs`**：
   - `load_recent_topic_traces(conn, project, topic_limit)`：取该 project 最近活跃的 N 个 topic_key
     （`SELECT topic_key, MAX(covered_to_event_id) m FROM topic_segments WHERE project=?1 GROUP BY topic_key ORDER BY m DESC LIMIT ?2`），
     每个调 `db::load_trace_by_topic_key`。
   - `render_topic_timeline_with_limit(traces, item_limit, char_limit) -> String`：仿 `sections/lessons.rs` 的预算渲染。
2. **`src/context/sections.rs`**：加 `mod topic_timeline;` + 必要 re-export。
3. **`src/context/policy.rs`**：加 `topic_timeline_char_limit` / `topic_timeline_item_limit`
   （默认值 + `REMEM_CONTEXT_TOPIC_TIMELINE_*` env override，仿现有字段）。
4. **`src/context/render.rs`**：在组装 sections 处（参照 lessons section 调用点）插入 timeline section，受预算。
5. **injection_gate**：section 文本自然进入 output hash，delta/suppress 无需改；确认不破坏 `injection_gate/tests.rs`。
6. **测试**：`render_topic_timeline_with_limit` 预算截断单测 + 一个 context load 集成测试
   （插入 topic_segments → load context → 断言含 timeline 段）。
7. **验证**：`cargo test --lib`（隔离 `REMEM_DATA_DIR`）；可选 LoCoMo before/after 看 Temporal/Multi-hop。

注意：`src/context/*` 在主树是别人在途改动密集区——务必在 worktree（cfadc85 干净 context）上做，勿动主树。
