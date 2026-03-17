# Zero-LLM 记忆系统重设计

## 核心思路

**当前问题**：remem 在 Stop hook 调用外部 LLM API（HTTP）来提取观察和生成摘要。这带来：
- 每次会话结束 2-5 秒延迟
- AI API 成本（~1.8K tokens/次）
- 网络失败导致记忆丢失
- 复杂的速率限制和重试逻辑

**核心洞察**：Claude 本身就是 LLM。它在对话中已经在"思考"。我们不需要另一个 LLM 来总结它做了什么——让 Claude 自己决定什么值得记住，通过 `save_memory` MCP tool 保存。

**新架构**：
```
自动捕获的结构化事件（what happened）
  + Claude 主动保存的记忆（what matters）
  + 规则化上下文生成（what to show）
  = 零外部 LLM 调用
```

---

## 数据模型

### 移除

| 表 | 原因 |
|----|------|
| `pending_observations` | 不再需要入队等待 LLM 处理 |
| `observations` | 被 events + memories 替代 |
| `session_summaries` | 被 sessions 统计替代 |
| `summarize_cooldown` | 无 LLM 调用，无需冷却 |
| `summarize_locks` | 无 LLM 调用，无需锁 |
| `ai_usage_events` | 无外部 AI 调用，无需追踪 |
| `jobs` | 无后台 AI 任务 |

### 新增

```sql
-- 结构化事件（自动捕获，规则化提取，不调用 LLM）
CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    project TEXT NOT NULL,
    event_type TEXT NOT NULL,   -- file_edit, file_create, file_read, bash, search, agent
    summary TEXT NOT NULL,      -- 规则生成："Edit src/db.rs:120-135"
    detail TEXT,                -- 关键内容（命令、查询、错误信息）
    files TEXT,                 -- JSON array: ["src/db.rs", "src/search.rs"]
    exit_code INTEGER,          -- bash 命令退出码
    created_at_epoch INTEGER NOT NULL
);
CREATE INDEX idx_events_session ON events(session_id, created_at_epoch);
CREATE INDEX idx_events_project ON events(project, created_at_epoch DESC);

-- 记忆（Claude 在对话中主动保存）
CREATE TABLE memories (
    id INTEGER PRIMARY KEY,
    session_id TEXT,
    project TEXT NOT NULL,
    topic_key TEXT,              -- 稳定主题标识，用于去重更新
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    memory_type TEXT NOT NULL,   -- decision, discovery, bugfix, preference, architecture
    files TEXT,                  -- JSON array: 相关文件
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    status TEXT DEFAULT 'active' -- active / archived
);
CREATE INDEX idx_memories_project ON memories(project, updated_at_epoch DESC);
CREATE INDEX idx_memories_topic ON memories(project, topic_key);

-- 记忆全文搜索
CREATE VIRTUAL TABLE memories_fts USING fts5(
    title, content, files,
    content='memories',
    content_rowid='id',
    tokenize='trigram'
);
-- triggers: INSERT/UPDATE/DELETE 同步到 memories_fts

-- 会话元数据（纯统计，无 LLM）
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    session_id TEXT UNIQUE NOT NULL,
    content_session_id TEXT,
    project TEXT,
    started_at_epoch INTEGER,
    ended_at_epoch INTEGER,
    event_count INTEGER DEFAULT 0,
    memory_count INTEGER DEFAULT 0,
    files_modified TEXT,         -- JSON array（从 events 聚合）
    status TEXT DEFAULT 'active'
);
```

### 保留

- `workstreams` + `workstream_sessions`（已实现，继续使用）
- `sdk_sessions`（hook 会话跟踪）

---

## 捕获流程

### PostToolUse hook（自动，规则化）

```
PostToolUse 触发
  ↓
解析 tool_name + tool_input + tool_result
  ↓
规则化提取元数据（正则，零 LLM）：
  Edit  → file_path, line_range        → "Edit src/db.rs:120-135"
  Write → file_path                    → "Create src/new_file.rs"
  Read  → file_path                    → "Read src/db.rs"
  Bash  → command, exit_code           → "Run `cargo test` (exit 0)"
  Grep  → pattern, path                → "Grep 'TODO' in src/"
  Glob  → pattern, count               → "Glob **/*.rs (12 files)"
  Agent → description                  → "Agent: 调研 Mem0 记忆流程"
  ↓
过滤噪音（同现有 should_skip 逻辑）：
  跳过 read-only（git status, ls, cat）
  跳过重复（同文件连续 Read）
  ↓
INSERT INTO events (structured, <1ms)
```

**规则化 summary 生成示例**：

```rust
fn event_summary(tool_name: &str, input: &Value, result: &Value) -> String {
    match tool_name {
        "Edit" => {
            let file = input["file_path"].as_str().unwrap_or("?");
            let short = file.rsplit('/').next().unwrap_or(file);
            format!("Edit {short}")
        }
        "Bash" => {
            let cmd = input["command"].as_str().unwrap_or("?");
            let short = cmd.chars().take(60).collect::<String>();
            let code = result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
            format!("Run `{short}` (exit {code})")
        }
        "Write" => {
            let file = input["file_path"].as_str().unwrap_or("?");
            format!("Create {file}")
        }
        _ => format!("{tool_name}")
    }
}
```

### save_memory MCP tool（Claude 主动）

Claude 在对话中判断值得记住的信息时调用。改进点：

```
Claude 调用 save_memory(title, content, topic_key?, memory_type?, files?)
  ↓
如果 topic_key 非空 且 同 project+topic_key 已存在：
  → UPDATE（更新 content + updated_at_epoch，版本迭代）
否则：
  → INSERT 新记忆
  ↓
同步更新 memories_fts
  ↓
写本地 markdown 备份（同现有逻辑）
```

**topic_key 去重示例**：

```
第一次：save_memory(title="FTS5 搜索问题", topic_key="fts5-search-bug", content="unicode61 不支持中文...")
  → INSERT id=1

第二次：save_memory(title="FTS5 搜索已修复", topic_key="fts5-search-bug", content="改用 trigram tokenizer...")
  → UPDATE id=1（同 topic_key，更新而非追加）
```

### Stop hook（会话结束，规则化）

```
Stop 触发
  ↓
聚合 session 统计（纯 SQL）：
  SELECT COUNT(*) as event_count FROM events WHERE session_id = ?
  SELECT DISTINCT files FROM events WHERE event_type IN ('file_edit','file_create')
  SELECT COUNT(*) as memory_count FROM memories WHERE session_id = ?
  ↓
UPDATE sessions SET ended_at_epoch=now, event_count=?, files_modified=?, memory_count=?
  ↓
WorkStream 生命周期清理（同现有逻辑）
  ↓
Compaction 生存写入（见下文）
```

---

## 检索流程

### SessionStart hook（规则化上下文生成）

不调用 LLM，纯 SQL + 模板渲染：

```
# [tools/remem@d431465db16f] 2026-03-15 2:09pm

## Recent Sessions
| Time | Duration | Files Modified | Events | Memories |
|------|----------|---------------|--------|----------|
| 03-15 12:00 | 1h30m | db.rs, search.rs, rate_limit.rs | 45 | 3 |
| 03-14 10:00 | 2h00m | mcp.rs, context.rs | 28 | 2 |

## Key Memories (recent 10)
| # | Type | Title | Updated |
|---|------|-------|---------|
| 1 | decision | FTS5 trigram + LIKE 回退方案 | 03-15 |
| 2 | bugfix | 中文搜索空结果:unicode61 tokenizer | 03-15 |
| 3 | architecture | WorkStream 三层模型 | 03-15 |

## Active WorkStreams
| # | Status | Title | Progress |
|---|--------|-------|----------|
| 1 | active | WorkStream 层实现 | 完成 7/7 步 |

## Recent Activity (last session)
- Edit src/db.rs (×3)
- Edit src/search.rs
- Edit src/db_query.rs
- Run `cargo test` (exit 0)
- Run `cargo build --release` (exit 0)
- Edit tests/rate_limit.rs
```

**上下文 token 预算**：
- Sessions 表格：~100 tokens
- Memories 表格：~200 tokens（10 条 × ~20 tokens）
- WorkStreams：~50 tokens
- Recent Activity：~100 tokens
- **总计：~450 tokens**（vs 当前 ~2000 tokens，节省 75%）

### MCP search tool

```
search(query, project?, memory_type?, limit?)
  ↓
tokens = query.split_whitespace()
any_short = tokens.any(|t| t.chars().count() < 3)
  ↓
if any_short:
  → LIKE 回退（同现有逻辑）
else:
  → FTS5 trigram MATCH
  ↓
返回 memories 列表（id, title, type, updated_at, content 截断）
```

### Compaction 生存

**问题**：SessionStart 注入的上下文在 `/compact` 后丢失。

**方案**：Stop hook 在会话结束时写入持久文件：

```
~/.claude/projects/{project-hash}/context.md
```

内容（自动生成，~200 tokens）：

```markdown
<!-- Auto-generated by remem. Do not edit. -->
## Project Memory (tools/remem)

### Key Decisions
- FTS5 trigram tokenizer for CJK support (2026-03-15)
- WorkStream lifecycle: active→paused(7d)→abandoned(30d) (2026-03-15)

### Current Work
- WorkStream: WorkStream 层实现 (completed)

### Recent Changes
- src/db.rs, src/search.rs, src/db_query.rs (2026-03-15)
```

Claude Code 自动加载 `~/.claude/projects/` 下的文件，compaction 后仍然可见。

---

## 引导 Claude 主动保存

在 MCP server instructions 中加入：

```
## When to save_memory
- 做出架构决策时（save_memory with type=decision）
- 发现并修复 bug 时（save_memory with type=bugfix）
- 发现重要的代码模式或约束时（save_memory with type=discovery）
- 了解到用户偏好时（save_memory with type=preference）

## When to search
- 修改已知项目代码前
- 遇到可能之前修过的 bug 时
- 用户问"之前怎么做的"时
```

---

## 生命周期管理

### 事件清理
```
events 表：保留 30 天，超期自动删除
  → Stop hook 或 SessionStart 时执行
  → DELETE FROM events WHERE created_at_epoch < now - 30*86400
```

### 记忆归档
```
memories 表：
  active → archived（180 天无访问）
  archived 记忆不出现在 context 输出，但 search 仍可找到
```

### topic_key 去重
```
同 project + topic_key 的记忆只保留最新版本
  → INSERT OR UPDATE 语义
  → 历史版本不保留（不需要 Zep 的 bitemporal）
```

---

## 模块变化

| 模块 | 操作 | 说明 |
|------|------|------|
| `src/summarize.rs` | **删除** | 不再需要 LLM 摘要 |
| `src/observe_flush.rs` | **删除** | 不再需要 LLM 提取 |
| `src/http.rs` | **删除** | 不再需要 HTTP AI 调用 |
| `prompts/summary.txt` | **删除** | 不再需要摘要 prompt |
| `src/event.rs` | **新建** | 结构化事件捕获 + 规则化提取 |
| `src/memory.rs` | **新建** | 记忆 CRUD + topic_key 去重 |
| `src/db.rs` | **重写** | 新 schema，移除旧表 |
| `src/context.rs` | **重写** | 规则化上下文生成，不依赖 observations |
| `src/mcp.rs` | **修改** | save_memory 增加 topic_key/memory_type，search 查 memories 表 |
| `src/hooks.rs` | **修改** | PostToolUse 写 events，Stop 写 session 统计 |
| `src/workstream.rs` | **保留** | 不变 |

---

## 对比：当前 vs 新设计

| 维度 | 当前 | 新设计 |
|------|------|--------|
| 外部 LLM 调用 | 每 session 1-3 次 | **零** |
| Stop hook 延迟 | 2-5 秒（HTTP API） | **<100ms**（纯 SQL） |
| 每 session 成本 | ~1.8K tokens（$0.01-0.05） | **$0** |
| 网络依赖 | 需要 AI API 可达 | **无** |
| 记忆质量 | LLM 自动提取（可能有幻觉） | **Claude 亲手写（精确）** |
| 数据完整性 | 3 层 gate 可能漏记 | **事件全量捕获** |
| Compaction 生存 | ❌ 丢失 | **✅ 持久文件** |
| 代码复杂度 | ~3000 行（含 summarize/flush/http） | **~1500 行** |
| 噪音过滤 | LLM 判断 | **规则过滤** |
| 同主题去重 | 无 | **topic_key 更新** |
