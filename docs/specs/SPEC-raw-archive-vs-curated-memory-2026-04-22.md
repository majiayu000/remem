---
spec: raw-archive-vs-curated-memory
status: active
date: 2026-04-22
owner: remem
---

# Raw Archive vs Curated Memory（采集与提炼解耦）

## 问题

当前 remem 的写入链路是：
`hook → summarize（AI 提炼 + cooldown/duplicate/skip 判断）→ promote（长度阈值 + 类型过滤）→ memories`。

结果：一段实际发生过的对话，只要在 summarize/promote 的任一步被跳过或过滤，就在 remem 里**彻底不可检索**。
已复现用例：trainer-hub 项目下的 VPS 采购对话存在于 `.claude/history.jsonl`（见行号 3727-3743），但
- `session_summaries` 里没有对应条目
- `memories` 里搜不到 RackNerd / OVHcloud / Hetzner 这些关键词

CLAUDE.md 已经记录了这是**致命设计缺陷**。本 spec 定义修复方案。

## 目标

把单层管道拆成两层：

1. **Raw archive（采集保证层）**
   - 每一条用户 prompt 和每一条 assistant 回复都必须落库。
   - 允许噪音，允许重复，允许低价值。
   - 唯一保证：**发生过的 → 可回忆**。

2. **Curated memory（提炼精选层）**
   - 只存 decision / discovery / preference / bugfix / architecture。
   - 由现有 summarize → promote 管道产出。
   - 允许跳过、去重、过滤。
   - 唯一保证：**高信号、低噪音**。

## 非目标

- 不保留 tool_input / tool_response 原文（已在 `pending_observations` / `events` 里）。
- 不替代 `session_summaries`，summary 依旧是人类可读的总结层。
- 不改 save_memory 语义。

## 数据模型

新表 `raw_messages`（schema v2）：

```sql
CREATE TABLE IF NOT EXISTS raw_messages (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    project TEXT NOT NULL,
    role TEXT NOT NULL,                 -- 'user' | 'assistant'
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,         -- FNV64 hex of content
    source TEXT NOT NULL,               -- 'transcript' | 'hook' | 'manual'
    branch TEXT,
    cwd TEXT,
    created_at_epoch INTEGER NOT NULL,
    UNIQUE(project, role, content_hash)
);

CREATE VIRTUAL TABLE IF NOT EXISTS raw_messages_fts USING fts5(
    content,
    content='raw_messages',
    content_rowid='id',
    tokenize='trigram'
);
-- + AI/AD/AU triggers analogous to memories_fts

CREATE INDEX idx_raw_messages_project_created
    ON raw_messages(project, created_at_epoch DESC);
CREATE INDEX idx_raw_messages_session
    ON raw_messages(session_id, created_at_epoch);
```

UNIQUE 做的是**项目内 role+content 去重**，同一段内容多次聊天只存一份，但时间戳按首次出现记录。

## 写入入口

只改 `Stop` hook（`remem summarize`）：

```
process_summary_job_input(input):
    parse SummarizeInput
    ----- NEW -----
    if transcript_path:
        drain_transcript_into_raw_archive(
            transcript_path, session_id, project, branch, cwd
        )
    elif last_assistant_message:
        insert_raw_message(
            session_id, project, 'assistant', last_assistant_message, 'hook'
        )
    ----- 原有逻辑 -----
    cooldown / duplicate / length guard ...
    AI summarize → promote
```

要点：
- Raw 写入**发生在所有 summary 短路之前**。summarize 被 cooldown、duplicate、AI skip、长度不足跳过，都不影响 raw 落库。
- Transcript drain 顺序扫描 jsonl，对每条 `type=user` 和 `type=assistant` 抽文本插入；UNIQUE 约束保证幂等，跨多次 Stop hook 重复 drain 不会膨胀。
- 写入失败只 `log::warn`，**不 propagate**——raw 落库是 best-effort，不能拖垮 summarize 主流程。

## 检索入口

### 新 MCP 工具：`search_raw`

```
search_raw(query, project?, role?, limit?, offset?, include_stale?) -> [RawHit]
```

- 纯 FTS5 查询 `raw_messages_fts`
- 返回字段：id, session_id, project, role, content (300-char preview), created_at, branch
- 文档中明确说明：这是**原始聊天内容**，可能含噪音，但保证"聊过的一定能搜到"。

### 修改 `search` 行为

`memory_service::search_memories` 在 curated 结果 < 3 条时，**自动 fallback** 查 raw_messages 并把结果以 `raw_hits` 字段一起返回（而不是替换 curated 结果）。MCP `search` 工具响应结构：

```json
{
  "results": [...],
  "raw_hits": [...],        // only present when fallback triggered
  "multi_hop": {...}        // optional, as today
}
```

前端提示词更新：当 curated 空但 raw_hits 非空时，agent 应说"这段内容只存在于 raw archive，没有被提炼为长期记忆"。

## 运维 / CLI

- `remem status` 增加一行 `raw_messages` 计数（和 sessions/summaries/memories 并列）。
- 不提供独立的 `remem raw` CLI 子命令（最小化改动；用 MCP `search_raw` 即可）。
- 不提供 GC（暂）；后续按 `created_at_epoch` 做按项目/时间窗裁剪时再加。

## 迁移

新增 migration `v002_raw_messages.sql`。不回填历史 transcript——历史就是历史，raw archive 从启用之日起生效。
不做兼容：任何旧读路径都不会触碰新表。

## 不做的事（显式拒绝）

- **不做 raw → memory 自动 promote**。Raw archive 的作用是兜底检索，不是 curated 层的输入源。如果要把 VPS 这类聊天升格为 preference，走 save_memory。
- **不用 raw_messages 做上下文注入**。只在显式 search_raw 时可见。
- **不加密 content**。现有 DB 已有可选加密，复用同一路径。

## 验证

1. 启用后，在 `/Users/lifcc/Desktop/code/AI/tools/trainer-hub` 项目发一条含 "VPS RackNerd" 的对话。
2. Stop hook 触发 → `raw_messages` 至少新增 1 条 role=user 记录。
3. 即便该会话 summary 被 cooldown/duplicate/skip 跳过，`search_raw("VPS RackNerd")` 也能命中。
4. `search("VPS")` 在 curated 无命中时，自动带回 raw_hits。

## 相关记忆

- decision#40469 `raw-archive-vs-curated-memory`（本 spec 的原始决策）
- CLAUDE.md 错误 2：save_memory 不会被调用
