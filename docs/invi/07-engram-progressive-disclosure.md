# Engram 渐进式披露机制深度调研

## 执行摘要

Engram 是一个 Go 实现的持久化记忆系统，为 AI 编码代理提供跨会话的记忆能力。其核心设计理念是**代理驱动的记忆提取** + **三层渐进式披露** + **主题键 UPSERT**，避免了外部 LLM 调用和复杂的向量数据库依赖。

**关键发现**：
1. **不依赖自动捕获**：Engram 信任代理主动调用 `mem_save`，不捕获原始工具调用
2. **主题键 UPSERT**：通过 `topic_key` 实现同主题记忆的演进式更新，而非无限追加
3. **Hash 去重**：15 分钟滚动窗口内的完全重复内容自动合并，增加 `duplicate_count`
4. **三层披露**：搜索 → 时间线 → 完整内容，token 效率优先
5. **FTS5 而非向量**：全文搜索覆盖 95% 场景，无需 ChromaDB/Pinecone

---

## 1. 如何提取：代理主动调用，不是被动捕获

### 1.1 提取触发机制

Engram **不自动捕获**原始工具调用（edit、bash、read 等），所有记忆来自代理的主动调用：

```
Agent 完成重要工作 → 主动调用 mem_save(title, content, type, topic_key)
                    ↓
                Engram 存储到 SQLite
```

**触发时机**（由 Memory Protocol 定义）：
- Bug 修复完成
- 架构或设计决策
- 非显而易见的代码库发现
- 配置变更或环境设置
- 模式建立（命名、结构、约定）
- 用户偏好或约束

**为什么不自动捕获？**
> "Raw tool calls (`edit: {file: "foo.go"}`, `bash: {command: "go build"}`) are noisy and pollute FTS5 search results. The agent's curated summaries are higher signal, more searchable, and don't bloat the database."
> — DOCS.md L634

### 1.2 提取粒度控制

代理通过 `type` 字段控制记忆类型：
- `decision` — 架构决策
- `bugfix` — Bug 修复
- `architecture` — 架构变更
- `pattern` — 模式/约定
- `config` — 配置变更
- `discovery` — 发现/学习
- `learning` — 通用学习

每个记忆包含：
- **title**：短标题，可搜索（如 "JWT auth middleware"）
- **content**：结构化内容（What/Why/Where/Learned）
- **type**：类型标签
- **topic_key**（可选）：主题键，用于 UPSERT
- **project**：项目名
- **scope**：`project`（默认）或 `personal`

### 1.3 与 Honcho 的对比

| 维度 | Engram | Honcho |
|------|--------|--------|
| 提取方式 | 代理主动调用 `mem_save` | 自动摄取 + 后台 Deriver |
| 提取粒度 | 代理决定（结构化摘要） | 消息级 + LLM 提取 |
| 成本 | 零（无外部 LLM） | 每百万 token $2 |
| 信噪比 | 高（代理筛选） | 中（需要后处理） |

---

## 2. 提取什么：结构化摘要 + 主题键

### 2.1 观察记录的内容类型

Engram 的 `Observation` 数据结构：

```go
type Observation struct {
    ID             int64   // 自增主键
    SyncID         string  // 跨设备同步 ID
    SessionID      string  // 会话 ID
    Type           string  // 类型标签
    Title          string  // 短标题
    Content        string  // 结构化内容
    ToolName       *string // 工具名（可选）
    Project        *string // 项目名
    Scope          string  // project | personal
    TopicKey       *string // 主题键（用于 UPSERT）
    NormalizedHash string  // 内容 hash（去重）
    RevisionCount  int     // 修订次数
    DuplicateCount int     // 重复次数
    LastSeenAt     *string // 最后出现时间
    CreatedAt      string  // 创建时间
    UpdatedAt      string  // 更新时间
    DeletedAt      *string // 软删除时间
}
```

### 2.2 Title + Content 的设计原则

**Title**：
- 短小精悍（< 50 字符）
- 可搜索的关键词
- 动词 + 对象（如 "Fixed N+1 query in UserList"）

**Content**：
- 结构化格式（强制）：
  ```
  **What**: [做了什么]
  **Why**: [为什么做]
  **Where**: [影响的文件/路径]
  **Learned**: [学到的教训/陷阱]
  ```

**示例**（来自 MCP 工具描述）：
```
title: "Switched from sessions to JWT"
type: "decision"
content: "**What**: Replaced express-session with jsonwebtoken for auth
**Why**: Session storage doesn't scale across multiple instances
**Where**: src/middleware/auth.ts, src/routes/login.ts
**Learned**: Must set httpOnly and secure flags on the cookie, refresh tokens need separate rotation logic"
```

### 2.3 Metadata 字段

- **project**：项目名（用于多项目隔离）
- **scope**：`project`（团队共享）或 `personal`（个人）
- **tool_name**：触发工具名（可选，通常为空）
- **session_id**：关联的会话 ID

### 2.4 主题键（topic_key）的生成规则

**目的**：让同一主题的记忆演进式更新，而非无限追加。

**生成逻辑**（`SuggestTopicKey` 函数）：

1. **推断主题家族**（基于 `type`）：
   - `architecture/design/adr/refactor` → `architecture`
   - `bug/bugfix/fix/incident/hotfix` → `bug`
   - `decision` → `decision`
   - `pattern/convention/guideline` → `pattern`
   - `config/setup/infra/ci` → `config`

2. **规范化标题**：
   - 去除 `<private>` 标签
   - 转小写
   - 空格替换为 `-`
   - 截断到 120 字符

3. **组合**：`{family}/{segment}`
   - 示例：`architecture/auth-model`、`bug/fts5-syntax-error`

**调用方式**：
```
1. Agent 调用 mem_suggest_topic_key(type, title, content)
2. Engram 返回建议的 topic_key
3. Agent 在后续 mem_save 中复用该 topic_key
```

---

## 3. 如何保存：三层披露 + Hash 去重 + 主题 UPSERT

### 3.1 三层披露的数据结构

**Tier 1: 搜索结果（紧凑）**
- 返回：ID + type + title + 300 字符预览
- Token 消耗：~100 tokens/条
- 用途：快速浏览，定位相关记忆

```
[1] #42 (bugfix) — Fixed FTS5 syntax error on special chars
    **What**: Wrapped each search term in quotes before passing to FTS5 MATCH
    **Why**: Users typing queries like 'fix auth bug' would crash because FTS5... [preview]
    2024-03-15 | project: engram | scope: project
```

**Tier 2: 时间线上下文（中等）**
- 返回：锚点记忆 + 前后 N 条记忆（默认 5）
- Token 消耗：~500 tokens
- 用途：理解记忆的上下文（同一会话内的前后工作）

```
Timeline around observation #42:

Before (5):
  [#40] (architecture) — Chose SQLite over Postgres
  [#41] (config) — Set up FTS5 virtual table
  ...

Focus:
  [#42] (bugfix) — Fixed FTS5 syntax error on special chars

After (5):
  [#43] (pattern) — Established sanitizeFTS() helper
  [#44] (decision) — Wrap all user queries in quotes
  ...
```

**Tier 3: 完整内容（按需）**
- 返回：完整的 `content` 字段（最多 50,000 字符）
- Token 消耗：变化（取决于内容长度）
- 用途：需要完整细节时

```
mem_get_observation(id: 42)
→ 返回完整的 What/Why/Where/Learned 内容
```

### 3.2 Hash 去重机制

**去重逻辑**（`AddObservation` 函数 L950-982）：

1. **计算 normalized_hash**：
   ```go
   func hashNormalized(content string) string {
       normalized := strings.ToLower(strings.Join(strings.Fields(content), " "))
       h := sha256.Sum256([]byte(normalized))
       return hex.EncodeToString(h[:])
   }
   ```

2. **15 分钟滚动窗口查重**：
   ```sql
   SELECT id FROM observations
   WHERE normalized_hash = ?
     AND project = ?
     AND scope = ?
     AND type = ?
     AND title = ?
     AND deleted_at IS NULL
     AND datetime(created_at) >= datetime('now', '-15 minutes')
   ORDER BY created_at DESC
   LIMIT 1
   ```

3. **如果找到重复**：
   ```sql
   UPDATE observations
   SET duplicate_count = duplicate_count + 1,
       last_seen_at = datetime('now'),
       updated_at = datetime('now')
   WHERE id = ?
   ```

**为什么 15 分钟？**
- 避免代理在短时间内重复保存相同内容
- 不阻止长期重复（可能是有意的重新记录）

### 3.3 主题键 UPSERT（同主题更新）

**UPSERT 逻辑**（`AddObservation` 函数 L903-948）：

1. **如果提供了 `topic_key`**，先查找同主题的最新记忆：
   ```sql
   SELECT id FROM observations
   WHERE topic_key = ?
     AND project = ?
     AND scope = ?
     AND deleted_at IS NULL
   ORDER BY datetime(updated_at) DESC, datetime(created_at) DESC
   LIMIT 1
   ```

2. **如果找到**，更新而非插入：
   ```sql
   UPDATE observations
   SET type = ?,
       title = ?,
       content = ?,
       tool_name = ?,
       topic_key = ?,
       normalized_hash = ?,
       revision_count = revision_count + 1,
       last_seen_at = datetime('now'),
       updated_at = datetime('now')
   WHERE id = ?
   ```

3. **如果未找到**，插入新记忆（`revision_count = 1`）

**示例场景**：
```
第 1 次：mem_save(title="Auth model", topic_key="architecture/auth-model")
        → 插入新记忆，revision_count=1

第 2 次：mem_save(title="Auth model v2", topic_key="architecture/auth-model")
        → 更新同一记忆，revision_count=2，内容替换为 v2

第 3 次：mem_save(title="Auth model final", topic_key="architecture/auth-model")
        → 更新同一记忆，revision_count=3，内容替换为 final
```

**关键点**：
- 同一 `project + scope + topic_key` 只保留一条记忆
- 历史版本不保留（只有最新版本 + `revision_count`）
- 不同 `topic_key` 不会互相覆盖

---

## 4. 如何更新：软删除 + 版本历史

### 4.1 软删除 vs 硬删除

**软删除**（默认）：
```sql
UPDATE observations
SET deleted_at = datetime('now')
WHERE id = ?
```
- 记录保留在数据库中
- 搜索和上下文查询自动过滤（`WHERE deleted_at IS NULL`）
- 可恢复（手动清空 `deleted_at`）

**硬删除**（可选）：
```sql
DELETE FROM observations WHERE id = ?
```
- 永久删除
- 无法恢复
- 用于敏感信息或错误记录

### 4.2 版本历史的保留策略

**Engram 不保留完整版本历史**，只保留：
- 最新版本的内容
- `revision_count`（修订次数）
- `duplicate_count`（重复次数）
- `last_seen_at`（最后出现时间）

**原因**：
- 简化存储（无需版本表）
- 记忆系统关注"当前状态"，不是"完整历史"
- Git 提供代码级历史，Engram 提供决策级历史

**如果需要历史**：
- 使用 Git Sync 功能（`.engram/chunks/` 目录）
- 每次 `engram sync` 创建新的 chunk 文件
- Chunk 文件不可变，提供时间点快照

### 4.3 时间线的重建逻辑

**时间线查询**（`Timeline` 函数）：

1. **获取锚点记忆**：
   ```sql
   SELECT * FROM observations WHERE id = ?
   ```

2. **获取前 N 条**（同一会话内）：
   ```sql
   SELECT * FROM observations
   WHERE session_id = ?
     AND datetime(created_at) < ?
     AND deleted_at IS NULL
   ORDER BY datetime(created_at) DESC
   LIMIT ?
   ```

3. **获取后 N 条**（同一会话内）：
   ```sql
   SELECT * FROM observations
   WHERE session_id = ?
     AND datetime(created_at) > ?
     AND deleted_at IS NULL
   ORDER BY datetime(created_at) ASC
   LIMIT ?
   ```

4. **组合返回**：
   ```json
   {
     "focus": { /* 锚点记忆 */ },
     "before": [ /* 前 N 条，按时间正序 */ ],
     "after": [ /* 后 N 条，按时间正序 */ ],
     "session_info": { /* 会话信息 */ },
     "total_in_range": 15
   }
   ```

**用途**：
- 理解某个决策的上下文
- 追溯 Bug 修复的前因后果
- 查看同一会话内的相关工作

---

## 5. 与竞品对比

### 5.1 Engram vs Honcho

| 维度 | Engram | Honcho |
|------|--------|--------|
| **架构** | Go 单二进制 + SQLite | Python/TS SDK + 云服务 |
| **提取方式** | 代理主动调用 | 自动摄取 + 后台 Deriver |
| **存储** | FTS5 全文搜索 | 向量 + 关系数据库 |
| **成本** | 零（本地） | $2/百万 token |
| **渐进式披露** | 3 层（搜索/时间线/完整） | 2 层（摘要/完整） |
| **主题管理** | topic_key UPSERT | 无（追加式） |
| **去重** | Hash + 15 分钟窗口 | 无明确机制 |
| **多方对话** | 不支持 | 支持（Peer 模型） |

### 5.2 Engram vs ENGRAM 论文（arxiv 2511.12960）

| 维度 | Engram（工具） | ENGRAM（论文） |
|------|---------------|---------------|
| **记忆类型** | 单一类型（观察） | 3 类型（episodic/semantic/procedural） |
| **路由** | 无（代理决定） | LLM 路由器（3-bit mask） |
| **检索** | FTS5 + 时间线 | 向量检索 + 类型分离 |
| **性能** | 未公开基准 | LoCoMo 77.55%，LongMemEval 71.40% |
| **实现** | 生产就绪 | 研究原型 |

### 5.3 Engram vs Mem0

| 维度 | Engram | Mem0 |
|------|--------|------|
| **提取** | 代理主动 | 自动 + LLM 提取 |
| **存储** | SQLite + FTS5 | Qdrant 向量 + 关系数据库 |
| **更新** | topic_key UPSERT | 追加式 |
| **部署** | 单二进制 | Python 服务 + 向量数据库 |
| **依赖** | 零 | Qdrant/ChromaDB/Pinecone |

---

## 6. 关键设计决策

### 6.1 为什么不自动捕获？

**Engram 的立场**：
> "Raw tool calls are noisy and pollute FTS5 search results. The agent's curated summaries are higher signal, more searchable, and don't bloat the database."

**优势**：
- 高信噪比（代理筛选）
- 可搜索性强（结构化内容）
- 数据库不膨胀

**劣势**：
- 依赖代理的"自觉性"
- 可能遗漏重要信息
- 需要 Memory Protocol 训练代理

### 6.2 为什么用 FTS5 而非向量？

**Engram 的立场**：
> "FTS5 covers 95% of use cases. No ChromaDB/Pinecone complexity."

**FTS5 的优势**：
- 零依赖（SQLite 内置）
- 精确匹配（关键词搜索）
- 快速（索引查询）
- 可解释（用户知道为什么匹配）

**向量的优势**：
- 语义相似（"JWT" 匹配 "token authentication"）
- 跨语言（英文查询匹配中文内容）
- 模糊匹配（拼写错误容忍）

**Engram 的选择**：
- 代码记忆场景关键词明确（文件名、函数名、技术栈）
- 结构化内容（What/Why/Where）提供足够上下文
- 向量搜索的收益不足以抵消复杂度

### 6.3 为什么用 topic_key UPSERT？

**问题**：无限追加导致记忆膨胀
```
第 1 天：决定用 JWT
第 2 天：JWT 改用 RS256
第 3 天：JWT 加入 refresh token
...
→ 搜索 "auth" 返回 10 条记忆，大部分过时
```

**Engram 的解决方案**：
```
topic_key = "architecture/auth-model"
→ 同一主题只保留最新版本
→ revision_count 记录演进次数
```

**优势**：
- 记忆数量可控
- 搜索结果不重复
- 最新信息优先

**劣势**：
- 历史版本丢失（需要 Git Sync 补偿）
- 需要代理正确使用 topic_key

---

## 7. 实现细节

### 7.1 数据库 Schema

```sql
CREATE TABLE observations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sync_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    type TEXT NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    tool_name TEXT,
    project TEXT,
    scope TEXT NOT NULL DEFAULT 'project',
    topic_key TEXT,
    normalized_hash TEXT,
    revision_count INTEGER NOT NULL DEFAULT 1,
    duplicate_count INTEGER NOT NULL DEFAULT 1,
    last_seen_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    deleted_at TEXT,
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

CREATE INDEX idx_obs_topic ON observations(topic_key, project, scope, updated_at DESC);
CREATE INDEX idx_obs_dedupe ON observations(normalized_hash, project, scope, type, title, created_at DESC);

CREATE VIRTUAL TABLE observations_fts USING fts5(
    title, content, tool_name, type, project,
    content='observations',
    content_rowid='id'
);
```

### 7.2 MCP 工具接口

**核心工具**（agent profile）：
- `mem_save` — 保存记忆
- `mem_search` — 搜索记忆
- `mem_context` — 获取最近上下文
- `mem_session_summary` — 会话摘要
- `mem_get_observation` — 获取完整内容
- `mem_suggest_topic_key` — 建议主题键

**管理工具**（admin profile）：
- `mem_update` — 更新记忆
- `mem_delete` — 删除记忆
- `mem_stats` — 统计信息
- `mem_timeline` — 时间线查询

### 7.3 Privacy 保护

**两层过滤**：
1. **插件层**（TypeScript）：
   ```typescript
   content = stripPrivateTags(content)
   ```

2. **存储层**（Go）：
   ```go
   func stripPrivateTags(s string) string {
       return privateTagRegex.ReplaceAllString(s, "[REDACTED]")
   }
   ```

**用法**：
```
Set up API with <private>sk-abc123</private>
→ 存储为：Set up API with [REDACTED]
```

---

## 8. 对 remem 的启示

### 8.1 可借鉴的设计

1. **主题键 UPSERT**：
   - remem 应支持 `topic_key` 字段
   - 同主题记忆演进式更新，而非追加
   - 保留 `revision_count` 追踪演进

2. **Hash 去重**：
   - 短时间窗口内的完全重复自动合并
   - 避免代理重复保存相同内容

3. **三层披露**：
   - 搜索返回紧凑结果（ID + 预览）
   - 时间线提供上下文
   - 按需加载完整内容

4. **结构化内容格式**：
   - 强制 What/Why/Where/Learned 格式
   - 提高可搜索性和可读性

### 8.2 不应照搬的部分

1. **不自动捕获**：
   - Engram 完全依赖代理主动调用
   - remem 应保留自动捕获作为后备
   - 混合模式：自动捕获 + 代理精炼

2. **不保留历史版本**：
   - Engram 只保留最新版本
   - remem 应考虑保留关键版本（如 stale 标记）

3. **FTS5 vs 向量**：
   - Engram 只用 FTS5
   - remem 可考虑混合（FTS5 + 轻量级向量）

### 8.3 remem 的差异化方向

1. **自动捕获 + LLM 提取**：
   - 不依赖代理的"自觉性"
   - 后台 LLM 提取关键信息
   - 代理可选择性精炼

2. **版本历史**：
   - 保留关键版本（stale 标记）
   - 支持版本对比
   - 时间旅行查询

3. **语义搜索**：
   - FTS5 + 轻量级向量（如 sqlite-vec）
   - 关键词精确匹配 + 语义模糊匹配
   - 混合排序

4. **工作流集成**：
   - Workstream 概念（Engram 无）
   - 多轮对话的上下文管理
   - 任务分解和追踪

---

## 9. 参考资料

### 9.1 Engram 项目

- **GitHub**: https://github.com/Gentleman-Programming/engram
- **文档**: DOCS.md（3113 行完整实现文档）
- **架构**: Go + SQLite + FTS5 + MCP
- **版本**: 0.1.0（活跃开发中）

### 9.2 学术论文

- **ENGRAM 论文**: [Effective, Lightweight Memory Orchestration for Conversational Agents](https://arxiv.org/html/2511.12960v1)
  - 三类型记忆（episodic/semantic/procedural）
  - LLM 路由器 + 向量检索
  - LoCoMo 77.55%，LongMemEval 71.40%

### 9.3 渐进式披露理论

- **Progressive Disclosure**: [Load Context Only When Needed](https://understandingdata.com/posts/progressive-disclosure-context/)
  - 分层组织（元数据 → 核心指令 → 详细资源）
  - 按需加载
  - 87% 成本节省

### 9.4 Honcho 对比

- **Honcho 文档**: https://docs.honcho.dev/
- **架构**: Python/TS SDK + 云服务
- **特点**: 自动摄取 + 多方对话 + 向量搜索
- **成本**: $2/百万 token

---

## 10. 结论

Engram 的核心价值在于**简单性** + **代理驱动** + **主题演进**：

1. **简单性**：单二进制 + SQLite + FTS5，零依赖
2. **代理驱动**：信任代理主动保存，不捕获噪音
3. **主题演进**：topic_key UPSERT，记忆可更新而非追加
4. **渐进式披露**：三层查询，token 效率优先

**对 remem 的启示**：
- 借鉴 topic_key UPSERT 和 Hash 去重
- 保留自动捕获（不完全依赖代理）
- 考虑混合搜索（FTS5 + 向量）
- 增强版本历史管理

**核心教训**：
> "记忆质量 > 记忆数量。代理的精炼摘要比原始工具调用更有价值。"
