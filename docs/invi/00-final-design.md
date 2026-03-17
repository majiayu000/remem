# remem 最强记忆系统架构设计

> 基于 10 份深度调研报告的完整架构设计
> 设计日期：2026-03-16
> 目标：做最强的 Claude Code 记忆系统，不是最便宜的

---

## 执行摘要

remem 的核心目标是成为 **Claude Code 的最强记忆系统**。通过深度调研 Mem0、Zep、Letta、Cursor、Augment、LangMem、Engram、学术论文、Rewind 和 Notion 共 10 个记忆系统，我们提取了最强设计并针对开发者场景优化。

### 核心设计理念

1. **质量优先于成本** — 用户不在乎 API 成本，只在乎记忆准确性
2. **自动捕获是主力** — 不依赖 Claude 主动调用 save_memory
3. **本地优先** — 隐私保护 + 零成本 + 完全可控
4. **简单架构** — 易于理解、修改、贡献

### 关键技术选型

| 组件 | 技术选择 | 理由 |
|------|---------|------|
| **LLM 提取** | Claude API (子进程调用) | 免费 + 高质量，Mem0 证明 26% 准确率提升 |
| **向量存储** | SQLite + sqlite-vec | 本地优先，零依赖，Letta 证明简单存储效果达 74% |
| **全文搜索** | SQLite FTS5 | 内置，快速，Engram 证明覆盖 95% 场景 |
| **时间模型** | Bitemporal (Zep) | 支持历史查询，审计追踪 |
| **去重策略** | Hash + 向量 + LLM | Mem0/Zep 三层漏斗，降低 90% 重复 |

### 与 10 份调研的对应关系

| 调研报告 | 核心借鉴 | 应用到 remem |
|---------|---------|-------------|
| **Mem0** | 双阶段提取 (事实提取 → 冲突解决) | 提取管道 Stage 2-3 |
| **Zep** | Bitemporal 时间模型 + 5 阶段流水线 | 时间戳设计 + 并发优化 |
| **Letta** | 三层内存 (Core/Archival/Recall) | 分层存储架构 |
| **Cursor** | 自动索引 + Merkle Tree 增量更新 | 增量索引机制 |
| **Augment** | 实时语义索引 + 自定义嵌入模型 | 检索策略 |
| **LangMem** | Subconscious 后台通道 | 后台提取不阻塞交互 |
| **Engram** | topic_key UPSERT + 渐进式披露 | 记忆演进 + 分层检索 |
| **学术论文** | 重要性评分 + 时间衰减 + 多维检索 | 检索排序算法 |
| **Rewind** | 本地优先 + 自动捕获 | 隐私保护策略 |
| **Notion** | 增量更新 (70% 计算量减少) | 哈希变更检测 |

### 预期效果

- **提取延迟**：< 100ms (事件捕获) + < 5s (LLM 批处理)
- **检索延迟**：< 200ms (混合检索)
- **记忆质量**：准确率 > 90% (参考 Mem0 26% 提升基线)
- **存储效率**：压缩率 > 10x (原始对话 → 结构化记忆)
- **成本控制**：< $0.05/对话 (批量处理 + 便宜模型)

---

## 整体架构

### 数据流图

```
┌─────────────────────────────────────────────────────────────────┐
│                     Claude Code 对话                              │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│  Stage 1: 原始事件捕获 (PostToolUse Hook)                         │
│  - 捕获工具调用 (edit/bash/read)                                  │
│  - 提取文件变更、命令输出                                          │
│  - 记录时间戳、会话 ID                                            │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│  Stage 2: 批量 LLM 提取 (后台异步)                                │
│  - 累积 N 条消息后触发                                            │
│  - LLM 提取：观察 + 事实 + 概念                                   │
│  - 重要性评分 (1-10)                                              │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│  Stage 3: 冲突检测与合并                                          │
│  - 向量搜索相似记忆 (Top-5)                                       │
│  - LLM 判断：ADD/UPDATE/DELETE/NONE                              │
│  - Hash 去重 (15 分钟窗口)                                        │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│  Stage 4: 重要性评分与分层                                        │
│  - 计算多维评分 (时间 + 重要性 + 频率)                            │
│  - 分配到层级：Core / Active / Archival                          │
│  - 更新访问计数和时间戳                                           │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│  Stage 5: 索引更新 (增量)                                         │
│  - 计算 embedding (本地模型)                                      │
│  - 更新向量索引 (sqlite-vec)                                      │
│  - 更新全文索引 (FTS5)                                            │
│  - 更新 Bitemporal 时间戳                                         │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                    SQLite 存储层                                  │
│  ┌──────────────┬──────────────┬──────────────┐                 │
│  │ observations │ embeddings   │ fts_index    │                 │
│  │ (结构化数据)  │ (向量)       │ (全文搜索)    │                 │
│  └──────────────┴──────────────┴──────────────┘                 │
└─────────────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                    检索与注入                                     │
│  1. 全文搜索 (FTS5) → 快速过滤                                    │
│  2. 向量搜索 (sqlite-vec) → 语义理解                              │
│  3. 多维重排序 (时间 + 重要性 + 相关性)                           │
│  4. LLM 生成上下文 (注入到 Claude prompt)                         │
└─────────────────────────────────────────────────────────────────┘
```

### 四个核心问题的答案

#### 1. 如何提取？

**触发时机**：
- **主通道**：PostToolUse hook 自动捕获每次工具调用
- **批量处理**：累积 5-10 条消息后触发 LLM 提取
- **手动补充**：save_memory 工具（可选）

**提取管道**（5 阶段）：
1. **原始捕获** — Hook 捕获工具调用和输出
2. **LLM 提取** — 批量调用 Claude 提取观察/事实/概念
3. **冲突检测** — 向量搜索 + LLM 判断重复/矛盾
4. **重要性评分** — LLM 评分 1-10 + 多维计算
5. **索引更新** — 增量更新向量和全文索引

**并发策略**（Zep 启发）：
- 向量搜索和 LLM 调用并行
- 使用 Semaphore 限流（默认 10 并发）
- 批量处理降低 API 调用成本

#### 2. 提取什么？

**内容类型**（MIRIX 启发）：
- **观察 (Observation)** — 具体事件，细粒度
  - 例："用户修改了 src/main.rs，添加了错误处理"
- **事实 (Fact)** — 提炼的知识，中粒度
  - 例："项目使用 Result<T, E> 而非 unwrap()"
- **概念 (Concept)** — 高层抽象，粗粒度
  - 例："用户偏好函数式编程风格"

**元数据**（Mem0 + Zep）：
```rust
struct Observation {
    id: Uuid,
    content: String,           // 自然语言内容
    observation_type: ObsType, // observation/fact/concept
    importance: f32,           // 1-10 评分

    // Bitemporal 时间戳
    valid_at: DateTime,        // 事实生效时间
    invalid_at: Option<DateTime>, // 事实失效时间
    created_at: DateTime,      // 系统创建时间
    expired_at: Option<DateTime>, // 系统删除时间

    // 访问统计
    access_count: u32,
    last_accessed: DateTime,

    // 关联信息
    session_id: String,
    project: Option<String>,
    files: Vec<String>,        // 相关文件

    // 索引
    embedding: Vec<f32>,       // 768 维向量
    hash: String,              // 内容哈希 (去重)
}
```

#### 3. 如何保存？

**分层存储**（Letta 三层 + HiMem 两层）：

```
Tier 0: Core Memory (compact-proof，始终可见)
├─ 用户身份 (user_profile)
├─ 项目上下文 (project_context)
└─ 当前任务 (current_workstream)
容量：< 2KB，始终注入到 prompt

Tier 1: Active Memory (高频访问，语义索引)
├─ 最近 7 天的观察
├─ 高重要性事实 (importance > 7)
└─ 频繁访问的概念 (access_count > 5)
容量：~100 条，按需检索

Tier 2: Archival Memory (长期存储，按需检索)
├─ 历史观察 (> 7 天)
├─ 低重要性事实 (importance < 7)
└─ 过期记忆 (invalid_at 已设置)
容量：无限，语义搜索
```

**存储后端**：
- **主数据库**：SQLite (observations 表)
- **向量索引**：sqlite-vec (本地向量搜索)
- **全文索引**：FTS5 (关键词搜索)
- **时间索引**：B-tree (时间范围查询)

**Schema 设计**：
```sql
-- 主表
CREATE TABLE observations (
    id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    observation_type TEXT NOT NULL,
    importance REAL NOT NULL,

    -- Bitemporal
    valid_at TEXT NOT NULL,
    invalid_at TEXT,
    created_at TEXT NOT NULL,
    expired_at TEXT,

    -- 统计
    access_count INTEGER DEFAULT 0,
    last_accessed TEXT,

    -- 关联
    session_id TEXT NOT NULL,
    project TEXT,
    files TEXT, -- JSON array

    -- 去重
    content_hash TEXT NOT NULL,

    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

-- 向量索引 (sqlite-vec)
CREATE VIRTUAL TABLE vec_observations USING vec0(
    observation_id TEXT PRIMARY KEY,
    embedding FLOAT[768]
);

-- 全文索引 (FTS5)
CREATE VIRTUAL TABLE fts_observations USING fts5(
    content,
    observation_type,
    files,
    content='observations',
    content_rowid='rowid'
);

-- 索引
CREATE INDEX idx_obs_time ON observations(valid_at DESC, created_at DESC);
CREATE INDEX idx_obs_importance ON observations(importance DESC);
CREATE INDEX idx_obs_session ON observations(session_id);
CREATE INDEX idx_obs_hash ON observations(content_hash);
```

#### 4. 如何更新？

**冲突检测**（Mem0 四种操作）：
```rust
enum MemoryOp {
    Add,      // 新信息，不存在于旧记忆
    Update,   // 信息有更多细节或矛盾
    Delete,   // 新信息与旧记忆矛盾
    None,     // 信息已存在且无需更新
}
```

**去重策略**（Zep 三层漏斗）：
1. **Hash 去重** — 15 分钟窗口内完全相同内容合并
2. **向量去重** — 余弦相似度 > 0.95 的候选
3. **LLM 判断** — 最终决定是否重复/矛盾

**更新逻辑**：
```rust
async fn resolve_conflict(
    new_obs: &Observation,
    existing: &[Observation],
) -> MemoryOp {
    // 1. Hash 去重
    if let Some(dup) = find_hash_duplicate(new_obs, existing) {
        dup.access_count += 1;
        dup.last_accessed = now();
        return MemoryOp::None;
    }

    // 2. 向量搜索相似记忆
    let similar = vector_search(&new_obs.embedding, 5).await?;

    // 3. LLM 判断
    let decision = llm_resolve_conflict(new_obs, &similar).await?;

    match decision {
        MemoryOp::Update => {
            // 设置旧记忆的 invalid_at
            similar[0].invalid_at = Some(new_obs.valid_at);
        }
        MemoryOp::Delete => {
            // 标记为过期
            similar[0].expired_at = Some(now());
        }
        _ => {}
    }

    decision
}
```

**遗忘机制**（MemoryBank + SynapticRAG）：
```rust
// 时间衰减函数
fn decay_score(obs: &Observation) -> f32 {
    let hours_since = (now() - obs.last_accessed).hours();
    let tau = 168.0 * (1.0 + obs.access_count as f32 / 10.0); // 基础 7 天
    (-hours_since / tau).exp()
}

// 遗忘条件
fn should_forget(obs: &Observation) -> bool {
    obs.importance < 3.0
        && decay_score(obs) < 0.1
        && obs.access_count == 0
}
```

---

## 分层设计

### Tier 0: Core Memory（始终可见）

**设计理念**（Letta Core Memory）：
- 始终注入到 Claude prompt 的 system message
- 紧凑表示（< 2KB），不占用太多 token
- 包含最关键的上下文信息

**内容类型**：
```rust
struct CoreMemory {
    user_profile: UserProfile,       // 用户身份和偏好
    project_context: ProjectContext, // 当前项目信息
    current_workstream: Option<Workstream>, // 当前任务
}

struct UserProfile {
    name: Option<String>,
    preferences: Vec<String>,  // ["偏好函数式编程", "避免 unwrap()"]
    constraints: Vec<String>,  // ["不使用 unsafe", "保持向后兼容"]
}

struct ProjectContext {
    name: String,
    language: String,          // "Rust"
    framework: Option<String>, // "Tokio"
    architecture: Vec<String>, // ["MCP server", "SQLite storage"]
}
```

**渲染格式**（Letta 启发）：
```xml
<core_memory>
<user_profile>
Name: Alice
Preferences:
- 偏好函数式编程风格
- 避免使用 unwrap()，使用 Result<T, E>
Constraints:
- 不使用 unsafe 代码
- 保持 API 向后兼容
</user_profile>

<project_context>
Project: remem
Language: Rust
Framework: Tokio (async runtime)
Architecture:
- MCP server for Claude Code integration
- SQLite for local storage
- sqlite-vec for vector search
</project_context>

<current_workstream>
Task: 设计最强记忆系统架构
Status: in_progress
Files: docs/invi/00-final-design.md
</current_workstream>
</core_memory>
```

**更新策略**：
- 手动更新（通过 save_memory 工具）
- 自动提取（从高重要性观察中提炼）
- 压缩机制（超过 2KB 时触发 LLM 压缩）

### Tier 1: Active Memory（高频访问）

**设计理念**（HiMem Note Memory）：
- 最近 7 天的观察 + 高重要性事实
- 语义索引，快速检索
- 压缩表示，加速检索

**容量限制**：
- 最多 100 条记忆
- 超过时按重要性 + 时间衰减淘汰

**检索策略**：
```rust
async fn search_active_memory(query: &str, limit: usize) -> Vec<Observation> {
    // 1. 时间过滤（最近 7 天）
    let recent = filter_by_time(now() - 7.days(), now());

    // 2. 混合检索
    let fts_results = fts_search(query, recent);
    let vec_results = vector_search(query, recent);

    // 3. 多维重排序
    let scored = rerank(fts_results, vec_results, |obs| {
        0.4 * recency_score(obs)
        + 0.3 * importance_score(obs)
        + 0.2 * relevance_score(obs, query)
        + 0.1 * frequency_score(obs)
    });

    scored.take(limit).collect()
}
```

**淘汰策略**：
```rust
fn evict_from_active() {
    let candidates = observations
        .filter(|o| o.created_at < now() - 7.days())
        .filter(|o| o.importance < 7.0)
        .sort_by_key(|o| combined_score(o));

    // 淘汰最低分的 20%
    let to_evict = candidates.take(candidates.len() / 5);
    for obs in to_evict {
        obs.tier = Tier::Archival; // 降级到 Tier 2
    }
}
```

### Tier 2: Archival Memory（长期存储）

**设计理念**（Letta Archival Memory）：
- 无容量限制，完整历史
- 按需检索，不主动加载
- 支持时间旅行查询

**检索策略**：
```rust
async fn search_archival_memory(
    query: &str,
    time_range: Option<(DateTime, DateTime)>,
    tags: Option<Vec<String>>,
    limit: usize,
) -> Vec<Observation> {
    let mut filters = vec![];

    // 时间过滤
    if let Some((start, end)) = time_range {
        filters.push(format!("valid_at BETWEEN '{}' AND '{}'", start, end));
    }

    // 标签过滤
    if let Some(tags) = tags {
        filters.push(format!("files LIKE '%{}%'", tags.join("%")));
    }

    // 排除已过期
    filters.push("expired_at IS NULL".to_string());

    // 语义搜索
    let results = vector_search_with_filters(query, filters, limit * 2).await?;

    // LLM 重排序（可选）
    if results.len() > limit {
        llm_rerank(query, results, limit).await
    } else {
        results
    }
}
```

**归档策略**：
```rust
// 定期归档（每天运行）
async fn archive_old_memories() {
    // 1. 低重要性 + 长时间未访问 → 归档
    let to_archive = observations
        .filter(|o| o.importance < 5.0)
        .filter(|o| o.last_accessed < now() - 30.days())
        .filter(|o| o.tier == Tier::Active);

    for obs in to_archive {
        obs.tier = Tier::Archival;
    }

    // 2. 压缩相似记忆
    let similar_groups = find_similar_clusters(0.9); // 余弦相似度 > 0.9
    for group in similar_groups {
        let merged = llm_merge_memories(group).await?;
        delete_observations(group);
        insert_observation(merged);
    }
}
```

### 层级转换规则

**提升到 Active**：
- 访问频率 > 5 次/周
- 重要性评分 > 7
- 最近 7 天内访问过

**降级到 Archival**：
- 超过 7 天未访问
- 重要性评分 < 5
- Active 层容量超限

**提升到 Core**：
- 重要性评分 = 10
- 用户显式标记（save_memory with core=true）
- 频繁访问（> 20 次）

---

## 提取管道

### Stage 1: 原始事件捕获

**Hook 实现**（PostToolUse）：
```rust
// src/hooks/post_tool_use.rs
pub async fn post_tool_use_hook(
    tool_name: &str,
    tool_input: &Value,
    tool_output: &Value,
    session_id: &str,
) -> Result<()> {
    // 1. 提取关键信息
    let event = RawEvent {
        tool_name: tool_name.to_string(),
        timestamp: Utc::now(),
        session_id: session_id.to_string(),
        files_modified: extract_files(tool_name, tool_input)?,
        command_output: extract_output(tool_name, tool_output)?,
    };

    // 2. 写入缓冲区（不阻塞）
    EVENT_BUFFER.lock().await.push(event);

    // 3. 检查是否触发批量处理
    if EVENT_BUFFER.lock().await.len() >= BATCH_SIZE {
        tokio::spawn(async {
            process_event_batch().await;
        });
    }

    Ok(())
}
```

**捕获范围**：
- **edit 工具** — 文件路径、变更内容
- **bash 工具** — 命令、退出码、输出
- **read 工具** — 文件路径
- **其他工具** — 工具名、参数

**过滤规则**：
```rust
fn should_capture(tool_name: &str) -> bool {
    match tool_name {
        "edit" | "bash" | "read" => true,
        "list_files" | "search_files" => false, // 噪音太大
        _ => true, // 默认捕获
    }
}
```

### Stage 2: 批量 LLM 提取

**触发条件**：
- 累积 5-10 条事件
- 或距离上次提取超过 5 分钟
- 或会话结束

**提取 Prompt**（Mem0 启发）：
```rust
const EXTRACTION_PROMPT: &str = r#"
你是一个记忆提取专家，负责从开发者的工作记录中提取关键信息。

# 输入
以下是最近的工具调用记录：

<events>
{events}
</events>

# 任务
提取以下三类信息：

1. **观察 (Observations)** — 具体事件
   - 用户做了什么操作
   - 修改了哪些文件
   - 执行了什么命令

2. **事实 (Facts)** — 提炼的知识
   - 项目使用的技术栈
   - 代码风格和约定
   - 架构决策

3. **概念 (Concepts)** — 高层抽象
   - 用户的偏好和习惯
   - 项目的设计理念
   - 重复出现的模式

# 输出格式
返回 JSON 数组，每个元素包含：
- type: "observation" | "fact" | "concept"
- content: 自然语言描述（< 200 字符）
- importance: 1-10 评分
- files: 相关文件列表（可选）

# 示例
{
  "memories": [
    {
      "type": "observation",
      "content": "用户修改了 src/main.rs，添加了错误处理逻辑",
      "importance": 6,
      "files": ["src/main.rs"]
    },
    {
      "type": "fact",
      "content": "项目使用 Result<T, E> 而非 unwrap() 处理错误",
      "importance": 8,
      "files": []
    },
    {
      "type": "concept",
      "content": "用户偏好函数式编程风格，避免可变状态",
      "importance": 9,
      "files": []
    }
  ]
}
"#;
```

**批量处理**：
```rust
async fn process_event_batch() -> Result<()> {
    let events = EVENT_BUFFER.lock().await.drain(..).collect::<Vec<_>>();

    // 1. 调用 LLM 提取
    let prompt = format_extraction_prompt(&events);
    let response = call_claude_api(&prompt).await?;
    let memories: Vec<ExtractedMemory> = serde_json::from_str(&response)?;

    // 2. 并行处理每个记忆
    let handles: Vec<_> = memories
        .into_iter()
        .map(|mem| tokio::spawn(process_single_memory(mem)))
        .collect();

    futures::future::join_all(handles).await;

    Ok(())
}
```

### Stage 3: 冲突检测与合并

**去重流程**（Zep 三层漏斗）：
```rust
async fn deduplicate_memory(new_mem: &ExtractedMemory) -> Result<MemoryOp> {
    // 1. Hash 去重（15 分钟窗口）
    let hash = compute_hash(&new_mem.content);
    if let Some(existing) = find_by_hash_recent(hash, 15 * 60).await? {
        existing.access_count += 1;
        existing.last_accessed = Utc::now();
        return Ok(MemoryOp::None);
    }

    // 2. 向量搜索相似记忆
    let embedding = compute_embedding(&new_mem.content).await?;
    let similar = vector_search(&embedding, 5, 0.95).await?;

    if similar.is_empty() {
        return Ok(MemoryOp::Add);
    }

    // 3. LLM 判断
    let decision = llm_resolve_conflict(new_mem, &similar).await?;

    Ok(decision)
}
```

**冲突解决 Prompt**（Mem0 启发）：
```rust
const CONFLICT_RESOLUTION_PROMPT: &str = r#"
你是一个记忆管理专家，负责判断新记忆与现有记忆的关系。

# 新记忆
{new_memory}

# 现有记忆
{existing_memories}

# 任务
判断新记忆应该如何处理：

1. **ADD** — 新信息，不存在于现有记忆中
2. **UPDATE** — 信息有更多细节，需要更新现有记忆
3. **DELETE** — 新信息与现有记忆矛盾，需要删除旧记忆
4. **NONE** — 信息已存在且准确，无需操作

# 输出格式
{
  "operation": "ADD" | "UPDATE" | "DELETE" | "NONE",
  "target_id": "existing_memory_id" (UPDATE/DELETE 时必填),
  "reason": "简短说明原因"
}
"#;
```

### Stage 4: 重要性评分与分层

**多维评分**（Generative Agents + 学术论文）：
```rust
fn compute_importance_score(obs: &Observation) -> f32 {
    // 1. LLM 评分（1-10）
    let llm_score = obs.importance;

    // 2. 时间近期性
    let hours_since = (Utc::now() - obs.created_at).num_hours() as f32;
    let recency = (-hours_since / 168.0).exp(); // 7 天衰减

    // 3. 访问频率
    let frequency = (obs.access_count as f32).ln_1p() / 5.0;

    // 4. 文件关联性
    let file_bonus = if obs.files.is_empty() { 0.0 } else { 0.1 };

    // 加权求和
    0.6 * llm_score / 10.0
        + 0.2 * recency
        + 0.1 * frequency
        + 0.1 * file_bonus
}
```

**分层分配**：
```rust
fn assign_tier(obs: &Observation) -> Tier {
    let score = compute_importance_score(obs);

    if obs.importance >= 9.0 || obs.access_count > 20 {
        Tier::Core
    } else if score > 0.7 || obs.created_at > Utc::now() - Duration::days(7) {
        Tier::Active
    } else {
        Tier::Archival
    }
}
```

### Stage 5: 索引更新

**增量更新**（Cursor Merkle Tree + Notion 哈希检测）：
```rust
async fn update_indexes(obs: &Observation) -> Result<()> {
    // 1. 计算 embedding（本地模型）
    let embedding = compute_embedding_local(&obs.content).await?;

    // 2. 更新向量索引
    sqlx::query!(
        "INSERT INTO vec_observations (observation_id, embedding) VALUES (?, ?)",
        obs.id,
        embedding
    )
    .execute(&pool)
    .await?;

    // 3. 更新全文索引（自动触发）
    // FTS5 会自动索引 observations 表的变更

    // 4. 更新时间索引（B-tree 自动维护）

    Ok(())
}
```

**本地 Embedding 模型**（Augment 启发）：
```rust
// 使用 fastembed 或 onnx runtime
async fn compute_embedding_local(text: &str) -> Result<Vec<f32>> {
    // 选项 1: fastembed (Rust 原生)
    let model = TextEmbedding::try_new(Default::default())?;
    let embeddings = model.embed(vec![text], None)?;
    Ok(embeddings[0].clone())

    // 选项 2: 调用本地 Ollama
    // let response = reqwest::get(format!("http://localhost:11434/api/embeddings?model=nomic-embed-text&prompt=", text)).await?;
    // Ok(response.json().await?)
}
```

---

## 检索策略

### 四阶段检索流程

remem 采用混合检索策略，结合 Zep 的并行检索、Cursor 的动态上下文发现和学术论文的多维评分。

```
┌─────────────────────────────────────────────────────────────────┐
│  Stage 1: 全文搜索 (FTS5)                                         │
│  - 关键词匹配                                                     │
│  - 快速过滤（< 10ms）                                             │
│  - 召回率优先                                                     │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│  Stage 2: 向量搜索 (sqlite-vec)                                   │
│  - 语义相似度匹配                                                 │
│  - 并行执行（与 Stage 1 同时）                                    │
│  - 精确率优先                                                     │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│  Stage 3: 多维重排序                                              │
│  - 时间近期性 (40%)                                               │
│  - 重要性评分 (30%)                                               │
│  - 语义相关性 (20%)                                               │
│  - 访问频率 (10%)                                                 │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│  Stage 4: LLM 生成上下文                                          │
│  - 压缩记忆为紧凑表示                                             │
│  - 生成自然语言摘要                                               │
│  - 注入到 Claude prompt                                           │
└─────────────────────────────────────────────────────────────────┘
```

### Stage 1: 全文搜索

**实现**（SQLite FTS5）：
```rust
async fn fts_search(query: &str, limit: usize) -> Result<Vec<Observation>> {
    // 1. 分词和查询扩展
    let expanded_query = expand_query(query); // "rust error" → "rust OR error OR 错误"

    // 2. FTS5 查询
    let results = sqlx::query_as!(
        Observation,
        r#"
        SELECT o.*
        FROM observations o
        JOIN fts_observations fts ON o.rowid = fts.rowid
        WHERE fts_observations MATCH ?
        AND o.expired_at IS NULL
        ORDER BY rank
        LIMIT ?
        "#,
        expanded_query,
        limit * 3 // 召回 3 倍候选
    )
    .fetch_all(&pool)
    .await?;

    Ok(results)
}
```

**查询扩展**（Engram 启发）：
```rust
fn expand_query(query: &str) -> String {
    let mut terms = vec![query.to_string()];

    // 1. 同义词扩展
    if query.contains("error") {
        terms.push("错误".to_string());
        terms.push("异常".to_string());
    }

    // 2. 文件名提取
    if let Some(file) = extract_filename(query) {
        terms.push(format!("files:\"{}\"", file));
    }

    // 3. 布尔组合
    terms.join(" OR ")
}
```

### Stage 2: 向量搜索

**实现**（sqlite-vec）：
```rust
async fn vector_search(
    query: &str,
    limit: usize,
    threshold: f32,
) -> Result<Vec<Observation>> {
    // 1. 计算查询向量
    let query_embedding = compute_embedding_local(query).await?;

    // 2. 向量搜索
    let results = sqlx::query!(
        r#"
        SELECT
            o.*,
            vec_distance_cosine(v.embedding, ?) as distance
        FROM vec_observations v
        JOIN observations o ON v.observation_id = o.id
        WHERE o.expired_at IS NULL
        AND vec_distance_cosine(v.embedding, ?) < ?
        ORDER BY distance
        LIMIT ?
        "#,
        query_embedding,
        query_embedding,
        1.0 - threshold, // 余弦距离 = 1 - 余弦相似度
        limit * 3
    )
    .fetch_all(&pool)
    .await?;

    Ok(results.into_iter().map(|r| r.into()).collect())
}
```

**并行执行**（Zep 启发）：
```rust
async fn hybrid_search(query: &str, limit: usize) -> Result<Vec<Observation>> {
    // 并行执行 FTS 和向量搜索
    let (fts_results, vec_results) = tokio::join!(
        fts_search(query, limit),
        vector_search(query, limit, 0.7)
    );

    // 合并结果（去重）
    let mut combined = HashMap::new();
    for obs in fts_results?.into_iter().chain(vec_results?) {
        combined.entry(obs.id).or_insert(obs);
    }

    Ok(combined.into_values().collect())
}
```

### Stage 3: 多维重排序

**评分函数**（Generative Agents + 学术论文）：
```rust
fn rerank_observations(
    observations: Vec<Observation>,
    query: &str,
    query_embedding: &[f32],
) -> Vec<Observation> {
    let mut scored: Vec<_> = observations
        .into_iter()
        .map(|obs| {
            let score = compute_retrieval_score(&obs, query, query_embedding);
            (obs, score)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scored.into_iter().map(|(obs, _)| obs).collect()
}

fn compute_retrieval_score(
    obs: &Observation,
    query: &str,
    query_embedding: &[f32],
) -> f32 {
    // 1. 时间近期性（指数衰减）
    let hours_since = (Utc::now() - obs.last_accessed).num_hours() as f32;
    let recency = (-hours_since / 168.0).exp(); // 7 天半衰期

    // 2. 重要性评分（归一化）
    let importance = obs.importance / 10.0;

    // 3. 语义相关性（余弦相似度）
    let relevance = cosine_similarity(&obs.embedding, query_embedding);

    // 4. 访问频率（对数归一化）
    let frequency = (obs.access_count as f32 + 1.0).ln() / 5.0;

    // 加权求和
    0.4 * recency + 0.3 * importance + 0.2 * relevance + 0.1 * frequency
}
```

**时间衰减公式**（MemoryBank）：
```rust
// 自适应衰减：访问越频繁，衰减越慢
fn adaptive_decay(obs: &Observation) -> f32 {
    let base_tau = 168.0; // 基础 7 天
    let tau = base_tau * (1.0 + obs.access_count as f32 / 10.0); // 最多延长到 70 天
    let hours_since = (Utc::now() - obs.last_accessed).num_hours() as f32;
    (-hours_since / tau).exp()
}
```

### Stage 4: LLM 生成上下文

**压缩策略**（Letta Recall Memory）：
```rust
async fn generate_context(
    observations: Vec<Observation>,
    query: &str,
    max_tokens: usize,
) -> Result<String> {
    // 1. 按层级分组
    let core = observations.iter().filter(|o| o.tier == Tier::Core);
    let active = observations.iter().filter(|o| o.tier == Tier::Active);

    // 2. Core Memory 完整展示
    let mut context = String::new();
    context.push_str("<relevant_memories>\n");
    for obs in core {
        context.push_str(&format!("- {}\n", obs.content));
    }

    // 3. Active Memory 压缩展示
    if active.count() > 10 {
        // 调用 LLM 压缩
        let compressed = llm_compress_memories(active.collect(), max_tokens / 2).await?;
        context.push_str(&compressed);
    } else {
        for obs in active {
            context.push_str(&format!("- {}\n", obs.content));
        }
    }

    context.push_str("</relevant_memories>\n");
    Ok(context)
}
```

**压缩 Prompt**：
```rust
const COMPRESSION_PROMPT: &str = r#"
你是一个记忆压缩专家，负责将多条记忆压缩为紧凑表示。

# 输入记忆
{memories}

# 任务
将上述记忆压缩为 3-5 条关键要点，保留最重要的信息。

# 输出格式
- 要点 1
- 要点 2
- 要点 3

# 要求
- 保留具体细节（文件名、技术栈、决策）
- 删除冗余和重复信息
- 使用简洁的自然语言
"#;
```

### 动态上下文发现

**渐进式披露**（Engram 三层检索）：
```rust
async fn progressive_retrieval(query: &str) -> Result<Vec<Observation>> {
    // Layer 1: 精确匹配（FTS5）
    let exact = fts_search(query, 5).await?;
    if exact.len() >= 3 {
        return Ok(exact);
    }

    // Layer 2: 语义搜索（向量）
    let semantic = vector_search(query, 10, 0.7).await?;
    if semantic.len() >= 5 {
        return Ok(semantic);
    }

    // Layer 3: 扩展搜索（降低阈值）
    let expanded = vector_search(query, 20, 0.5).await?;
    Ok(expanded)
}
```

**关联记忆扩展**（Cursor Dynamic Context）：
```rust
async fn expand_related_memories(
    seed: &Observation,
    max_depth: usize,
) -> Result<Vec<Observation>> {
    let mut visited = HashSet::new();
    let mut results = vec![seed.clone()];
    visited.insert(seed.id);

    for depth in 0..max_depth {
        let current_layer = results.clone();
        for obs in current_layer {
            // 1. 文件关联
            let file_related = find_by_files(&obs.files).await?;

            // 2. 时间关联（前后 1 小时）
            let time_related = find_by_time_range(
                obs.created_at - Duration::hours(1),
                obs.created_at + Duration::hours(1),
            )
            .await?;

            // 3. 语义关联
            let semantic_related = vector_search_by_embedding(&obs.embedding, 5, 0.8).await?;

            // 合并并去重
            for related in file_related
                .into_iter()
                .chain(time_related)
                .chain(semantic_related)
            {
                if visited.insert(related.id) {
                    results.push(related);
                }
            }
        }
    }

    Ok(results)
}
```

### 性能优化

**缓存策略**（Augment 实时索引）：
```rust
// 查询结果缓存（LRU）
static QUERY_CACHE: Lazy<Mutex<LruCache<String, Vec<Observation>>>> =
    Lazy::new(|| Mutex::new(LruCache::new(100)));

async fn cached_search(query: &str, limit: usize) -> Result<Vec<Observation>> {
    let cache_key = format!("{}:{}", query, limit);

    // 1. 检查缓存
    if let Some(cached) = QUERY_CACHE.lock().await.get(&cache_key) {
        return Ok(cached.clone());
    }

    // 2. 执行搜索
    let results = hybrid_search(query, limit).await?;

    // 3. 写入缓存
    QUERY_CACHE.lock().await.put(cache_key, results.clone());

    Ok(results)
}
```

**索引预热**（Notion 增量更新）：
```rust
// 启动时预加载热点记忆
async fn warmup_indexes() -> Result<()> {
    // 1. 加载 Active Memory 到内存
    let active = sqlx::query_as!(
        Observation,
        "SELECT * FROM observations WHERE tier = 'active' ORDER BY importance DESC LIMIT 100"
    )
    .fetch_all(&pool)
    .await?;

    // 2. 预计算 embedding
    for obs in active {
        compute_embedding_local(&obs.content).await?;
    }

    // 3. 预热 FTS5 索引
    sqlx::query!("SELECT * FROM fts_observations WHERE fts_observations MATCH 'rust' LIMIT 1")
        .fetch_optional(&pool)
        .await?;

    Ok(())
}
```

---

## 更新机制

### 记忆演进模型

remem 采用 **topic_key UPSERT** 模式（Engram 启发），支持记忆的增量更新而非简单追加。

**核心理念**：
- 同一主题的记忆应该**演进**而非**堆积**
- 新信息应该**合并**到旧记忆，而非创建重复条目
- 矛盾信息应该**替换**旧记忆，保留历史版本

### 冲突检测算法

**三层去重漏斗**（Mem0 + Zep）：
```rust
async fn detect_conflict(new_mem: &ExtractedMemory) -> Result<ConflictResolution> {
    // Layer 1: Hash 去重（15 分钟窗口）
    let hash = compute_hash(&new_mem.content);
    if let Some(existing) = find_by_hash_recent(hash, 15 * 60).await? {
        return Ok(ConflictResolution::Duplicate(existing.id));
    }

    // Layer 2: 向量相似度（Top-5, threshold > 0.95）
    let embedding = compute_embedding_local(&new_mem.content).await?;
    let similar = vector_search_by_embedding(&embedding, 5, 0.95).await?;

    if similar.is_empty() {
        return Ok(ConflictResolution::Add);
    }

    // Layer 3: LLM 判断
    let decision = llm_resolve_conflict(new_mem, &similar).await?;

    Ok(decision)
}
```

**冲突解决决策树**：
```rust
enum ConflictResolution {
    Add,                          // 新信息，直接添加
    Duplicate(Uuid),              // 完全重复，增加访问计数
    Update { old_id: Uuid, merge: MergeStrategy }, // 更新现有记忆
    Delete(Uuid),                 // 矛盾信息，删除旧记忆
}

enum MergeStrategy {
    Replace,      // 完全替换
    Append,       // 追加细节
    Merge,        // LLM 合并
}
```

### LLM 冲突解决

**Prompt 设计**（Mem0 启发）：
```rust
const CONFLICT_RESOLUTION_PROMPT: &str = r#"
你是一个记忆管理专家，负责判断新记忆与现有记忆的关系。

# 新记忆
Content: {new_content}
Type: {new_type}
Importance: {new_importance}

# 现有记忆（按相似度排序）
{existing_memories}

# 判断规则
1. **ADD** — 新信息，不存在于现有记忆中
   - 例：新记忆提到新文件，现有记忆没有
2. **UPDATE** — 新信息有更多细节或更新
   - 例：新记忆说"用户偏好 async/await"，旧记忆说"用户偏好异步编程"
3. **DELETE** — 新信息与现有记忆矛盾
   - 例：新记忆说"项目不使用 ORM"，旧记忆说"项目使用 Diesel ORM"
4. **NONE** — 信息已存在且准确
   - 例：新记忆和旧记忆表达相同内容

# 输出格式
{
  "operation": "ADD" | "UPDATE" | "DELETE" | "NONE",
  "target_id": "uuid" (UPDATE/DELETE 时必填),
  "merge_strategy": "replace" | "append" | "merge" (UPDATE 时必填),
  "reason": "简短说明原因（< 50 字符）"
}
"#;
```

**执行更新**：
```rust
async fn execute_conflict_resolution(
    new_mem: &ExtractedMemory,
    resolution: ConflictResolution,
) -> Result<()> {
    match resolution {
        ConflictResolution::Add => {
            insert_observation(new_mem).await?;
        }
        ConflictResolution::Duplicate(id) => {
            sqlx::query!(
                "UPDATE observations SET access_count = access_count + 1, last_accessed = ? WHERE id = ?",
                Utc::now(),
                id
            )
            .execute(&pool)
            .await?;
        }
        ConflictResolution::Update { old_id, merge } => {
            match merge {
                MergeStrategy::Replace => {
                    // 设置旧记忆的 invalid_at
                    sqlx::query!(
                        "UPDATE observations SET invalid_at = ? WHERE id = ?",
                        Utc::now(),
                        old_id
                    )
                    .execute(&pool)
                    .await?;
                    insert_observation(new_mem).await?;
                }
                MergeStrategy::Append => {
                    let old = get_observation(old_id).await?;
                    let merged_content = format!("{}; {}", old.content, new_mem.content);
                    update_observation_content(old_id, &merged_content).await?;
                }
                MergeStrategy::Merge => {
                    let old = get_observation(old_id).await?;
                    let merged = llm_merge_memories(&old, new_mem).await?;
                    update_observation_content(old_id, &merged).await?;
                }
            }
        }
        ConflictResolution::Delete(id) => {
            sqlx::query!(
                "UPDATE observations SET expired_at = ? WHERE id = ?",
                Utc::now(),
                id
            )
            .execute(&pool)
            .await?;
        }
    }

    Ok(())
}
```

### Bitemporal 时间模型

**双时间轴**（Zep 启发）：
```rust
struct Observation {
    // 业务时间（事实生效时间）
    valid_at: DateTime<Utc>,      // 事实何时生效
    invalid_at: Option<DateTime<Utc>>, // 事实何时失效

    // 系统时间（记录创建时间）
    created_at: DateTime<Utc>,    // 记录何时创建
    expired_at: Option<DateTime<Utc>>, // 记录何时删除
}
```

**时间旅行查询**：
```rust
// 查询"2024-01-01 时用户知道什么"
async fn query_at_time(time: DateTime<Utc>) -> Result<Vec<Observation>> {
    sqlx::query_as!(
        Observation,
        r#"
        SELECT *
        FROM observations
        WHERE valid_at <= ?
        AND (invalid_at IS NULL OR invalid_at > ?)
        AND created_at <= ?
        AND (expired_at IS NULL OR expired_at > ?)
        "#,
        time, time, time, time
    )
    .fetch_all(&pool)
    .await
}
```

### 遗忘机制

**自动遗忘**（MemoryBank + SynapticRAG）：
```rust
// 每天运行一次
async fn forget_old_memories() -> Result<()> {
    let candidates = sqlx::query_as!(
        Observation,
        r#"
        SELECT *
        FROM observations
        WHERE importance < 3.0
        AND access_count = 0
        AND last_accessed < datetime('now', '-30 days')
        AND tier = 'archival'
        "#
    )
    .fetch_all(&pool)
    .await?;

    for obs in candidates {
        if should_forget(&obs) {
            sqlx::query!(
                "UPDATE observations SET expired_at = ? WHERE id = ?",
                Utc::now(),
                obs.id
            )
            .execute(&pool)
            .await?;
        }
    }

    Ok(())
}

fn should_forget(obs: &Observation) -> bool {
    let decay = adaptive_decay(obs);
    obs.importance < 3.0 && decay < 0.05 && obs.access_count == 0
}
```

**手动遗忘**（用户触发）：
```rust
// MCP 工具：forget_memory
pub async fn forget_memory(pattern: &str) -> Result<usize> {
    let matches = fts_search(pattern, 100).await?;

    // 显示候选并请求确认
    println!("找到 {} 条匹配记忆：", matches.len());
    for (i, obs) in matches.iter().enumerate() {
        println!("{}. {}", i + 1, obs.content);
    }

    // 标记为过期
    let count = sqlx::query!(
        "UPDATE observations SET expired_at = ? WHERE id IN (?)",
        Utc::now(),
        matches.iter().map(|o| o.id).collect::<Vec<_>>()
    )
    .execute(&pool)
    .await?
    .rows_affected();

    Ok(count as usize)
}
```

---

## 技术选型

### LLM 提取引擎

**选择：Claude API（子进程调用）**

**理由**：
- **免费** — Claude Code 用户已有 API 访问权限
- **高质量** — Mem0 证明 LLM 提取比规则提取准确率高 26%
- **简单集成** — 子进程调用，无需额外依赖

**替代方案对比**：
| 方案 | 优点 | 缺点 | 结论 |
|------|------|------|------|
| Claude API | 免费、高质量 | 需要网络 | ✅ 首选 |
| 本地 LLM (Ollama) | 完全离线 | 质量差、资源占用高 | ❌ 备选 |
| 规则提取 | 零成本、快速 | 准确率低、维护成本高 | ❌ 不采用 |

**实现**：
```rust
async fn call_claude_api(prompt: &str) -> Result<String> {
    let output = Command::new("claude")
        .arg("--api")
        .arg("--model")
        .arg("claude-3-haiku-20240307") // 便宜模型
        .arg("--prompt")
        .arg(prompt)
        .output()
        .await?;

    Ok(String::from_utf8(output.stdout)?)
}
```

### 向量存储

**选择：SQLite + sqlite-vec**

**理由**：
- **本地优先** — 零依赖，隐私保护
- **简单架构** — 单文件数据库，易于备份和迁移
- **性能足够** — Letta 证明简单存储在 LoCoMo 达到 74% 准确率
- **成本为零** — 无需外部服务

**替代方案对比**：
| 方案 | 优点 | 缺点 | 结论 |
|------|------|------|------|
| SQLite + sqlite-vec | 本地、零依赖、简单 | 扩展性有限（< 100 万条） | ✅ 首选 |
| Qdrant | 高性能、分布式 | 需要额外服务、复杂 | ❌ 过度设计 |
| Pinecone | 托管服务、免费额度 | 隐私问题、网络依赖 | ❌ 违背本地优先 |
| ChromaDB | 简单 API | Python 依赖、性能一般 | ❌ 跨语言调用复杂 |

**容量估算**：
- 每条记忆：~1KB（内容 + 元数据）+ 3KB（768 维 float32 向量）= 4KB
- 10 万条记忆：400MB
- 100 万条记忆：4GB（仍在 SQLite 性能范围内）

### 全文搜索

**选择：SQLite FTS5**

**理由**：
- **内置** — SQLite 自带，无需额外依赖
- **快速** — Engram 证明 FTS5 覆盖 95% 搜索场景
- **中文支持** — 支持 Unicode 分词

**配置**：
```sql
CREATE VIRTUAL TABLE fts_observations USING fts5(
    content,
    observation_type,
    files,
    tokenize='unicode61 remove_diacritics 2' -- Unicode 分词 + 去音调
);
```

### Embedding 模型

**选择：本地模型（fastembed 或 Ollama）**

**理由**：
- **零成本** — 无 API 调用费用
- **隐私保护** — 数据不离开本地
- **低延迟** — 无网络往返

**模型选择**：
| 模型 | 维度 | 速度 | 质量 | 结论 |
|------|------|------|------|------|
| nomic-embed-text | 768 | 快 | 高 | ✅ 首选 |
| all-MiniLM-L6-v2 | 384 | 很快 | 中 | ✅ 备选（资源受限） |
| text-embedding-3-small | 1536 | 慢（API） | 很高 | ❌ 违背本地优先 |

**实现**：
```rust
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};

lazy_static! {
    static ref EMBEDDING_MODEL: TextEmbedding = {
        TextEmbedding::try_new(InitOptions {
            model_name: EmbeddingModel::NomicEmbedTextV15,
            show_download_progress: true,
            ..Default::default()
        })
        .expect("Failed to load embedding model")
    };
}

async fn compute_embedding_local(text: &str) -> Result<Vec<f32>> {
    let embeddings = EMBEDDING_MODEL.embed(vec![text], None)?;
    Ok(embeddings[0].clone())
}
```

### 数据库 Schema

**选择：单数据库多表设计**

**Schema**：
```sql
-- 主表：观察记忆
CREATE TABLE observations (
    id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    observation_type TEXT NOT NULL CHECK(observation_type IN ('observation', 'fact', 'concept')),
    importance REAL NOT NULL CHECK(importance BETWEEN 1.0 AND 10.0),
    tier TEXT NOT NULL CHECK(tier IN ('core', 'active', 'archival')),

    -- Bitemporal 时间戳
    valid_at TEXT NOT NULL,
    invalid_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    expired_at TEXT,

    -- 访问统计
    access_count INTEGER NOT NULL DEFAULT 0,
    last_accessed TEXT NOT NULL DEFAULT (datetime('now')),

    -- 关联信息
    session_id TEXT NOT NULL,
    project TEXT,
    files TEXT, -- JSON array

    -- 去重
    content_hash TEXT NOT NULL,

    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

-- 会话表
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    ended_at TEXT,
    project TEXT,
    summary TEXT
);

-- 向量索引
CREATE VIRTUAL TABLE vec_observations USING vec0(
    observation_id TEXT PRIMARY KEY,
    embedding FLOAT[768]
);

-- 全文索引
CREATE VIRTUAL TABLE fts_observations USING fts5(
    content,
    observation_type,
    files,
    content='observations',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

-- 性能索引
CREATE INDEX idx_obs_time ON observations(valid_at DESC, created_at DESC);
CREATE INDEX idx_obs_importance ON observations(importance DESC);
CREATE INDEX idx_obs_tier ON observations(tier, importance DESC);
CREATE INDEX idx_obs_session ON observations(session_id);
CREATE INDEX idx_obs_hash ON observations(content_hash);
CREATE INDEX idx_obs_access ON observations(last_accessed DESC, access_count DESC);
```

### 并发模型

**选择：Tokio 异步运行时**

**理由**：
- **高并发** — Zep 证明并行处理可降低 75% 延迟
- **资源高效** — 异步 I/O 减少线程开销
- **生态成熟** — Rust 异步生态标准

**并发策略**：
```rust
use tokio::sync::Semaphore;

// 限流器：最多 10 个并发 LLM 调用
static LLM_SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(10));

async fn batch_extract_memories(events: Vec<RawEvent>) -> Result<Vec<ExtractedMemory>> {
    let handles: Vec<_> = events
        .chunks(5) // 每批 5 个事件
        .map(|chunk| {
            tokio::spawn(async move {
                let _permit = LLM_SEMAPHORE.acquire().await.unwrap();
                extract_from_events(chunk).await
            })
        })
        .collect();

    let results = futures::future::join_all(handles).await;
    Ok(results.into_iter().flatten().flatten().collect())
}
```

---

## 实现路线图

### Phase 1: MVP（2 周）

**目标**：验证核心假设，实现最小可用系统

**功能范围**：
- ✅ PostToolUse hook 捕获事件
- ✅ 批量 LLM 提取（观察 + 事实）
- ✅ SQLite 存储（单表，无向量）
- ✅ 简单检索（FTS5 全文搜索）
- ✅ MCP 工具：search_memory

**技术债务**（可接受）：
- 无向量搜索（仅 FTS5）
- 无去重（允许重复）
- 无分层（所有记忆平等）
- 无遗忘机制

**验证指标**：
- 提取延迟 < 10s（批量 10 条事件）
- 检索延迟 < 500ms
- 用户主观评价：记忆是否有用？

**交付物**：
```
src/
├── hooks/
│   └── post_tool_use.rs       # 事件捕获
├── extraction/
│   └── llm_extractor.rs       # LLM 提取
├── storage/
│   └── sqlite_store.rs        # SQLite 存储
├── retrieval/
│   └── fts_search.rs          # 全文搜索
└── mcp/
    └── tools.rs               # MCP 工具
```

### Phase 2: 质量提升（2 周）

**目标**：提升记忆质量，减少噪音

**功能范围**：
- ✅ 向量搜索（sqlite-vec）
- ✅ 三层去重（Hash + 向量 + LLM）
- ✅ 冲突检测与合并
- ✅ 重要性评分
- ✅ Bitemporal 时间模型

**技术债务清理**：
- 实现向量索引
- 实现去重逻辑
- 实现冲突解决

**验证指标**：
- 重复率 < 10%（去重效果）
- 准确率 > 85%（人工标注 100 条）
- 检索相关性 > 80%（Top-5 命中率）

**交付物**：
```
src/
├── extraction/
│   ├── deduplication.rs       # 去重
│   └── conflict_resolution.rs # 冲突解决
├── storage/
│   └── vector_index.rs        # 向量索引
└── retrieval/
    └── hybrid_search.rs       # 混合检索
```

### Phase 3: 性能优化（1 周）

**目标**：降低延迟，提升吞吐

**功能范围**：
- ✅ 并行提取（Tokio）
- ✅ 查询缓存（LRU）
- ✅ 索引预热
- ✅ 批量写入优化

**优化目标**：
- 提取延迟：10s → 3s（并行处理）
- 检索延迟：500ms → 100ms（缓存 + 索引）
- 吞吐量：10 events/s → 50 events/s

**交付物**：
```
src/
├── extraction/
│   └── parallel_extractor.rs  # 并行提取
├── retrieval/
│   └── cache.rs               # 查询缓存
└── storage/
    └── batch_writer.rs        # 批量写入
```

### Phase 4: 高级特性（2 周）

**目标**：完整功能，生产就绪

**功能范围**：
- ✅ 三层内存（Core/Active/Archival）
- ✅ 遗忘机制（自动 + 手动）
- ✅ 渐进式披露
- ✅ 关联记忆扩展
- ✅ 时间旅行查询
- ✅ MCP 工具完整集：
  - search_memory（搜索）
  - save_memory（手动保存）
  - forget_memory（遗忘）
  - timeline（时间线）
  - get_observations（详情）

**交付物**：
```
src/
├── memory/
│   ├── core_memory.rs         # Core Memory
│   ├── active_memory.rs       # Active Memory
│   └── archival_memory.rs     # Archival Memory
├── retrieval/
│   ├── progressive_disclosure.rs # 渐进式披露
│   └── related_expansion.rs   # 关联扩展
└── mcp/
    └── tools.rs               # 完整 MCP 工具集
```

### 里程碑时间表

```
Week 1-2:  Phase 1 (MVP)
Week 3-4:  Phase 2 (质量提升)
Week 5:    Phase 3 (性能优化)
Week 6-7:  Phase 4 (高级特性)
Week 8:    测试 + 文档 + 发布
```

---

## 性能指标

### 延迟指标

| 操作 | 目标延迟 | 测量方法 |
|------|---------|---------|
| **事件捕获** | < 10ms | PostToolUse hook 执行时间 |
| **批量提取** | < 5s | 10 条事件 → LLM 提取完成 |
| **单次检索** | < 200ms | 查询 → 返回 Top-10 结果 |
| **冲突检测** | < 1s | 向量搜索 + LLM 判断 |
| **索引更新** | < 100ms | 计算 embedding + 写入索引 |

### 质量指标

| 指标 | 目标值 | 测量方法 |
|------|--------|---------|
| **提取准确率** | > 90% | 人工标注 100 条，计算 precision |
| **检索相关性** | > 85% | Top-5 命中率（人工评估） |
| **去重效果** | < 10% 重复率 | 相似度 > 0.95 的记忆占比 |
| **遗忘准确性** | > 95% | 被遗忘的记忆中无用记忆占比 |

### 资源指标

| 资源 | 目标值 | 测量方法 |
|------|--------|---------|
| **内存占用** | < 200MB | 常驻内存（不含 embedding 模型） |
| **磁盘占用** | < 100MB/月 | 数据库文件大小增长 |
| **CPU 占用** | < 5% | 后台提取时平均 CPU 使用率 |
| **API 成本** | < $0.05/对话 | Claude API 调用费用 |

### 扩展性指标

| 场景 | 目标性能 | 测量方法 |
|------|---------|---------|
| **10 万条记忆** | 检索 < 300ms | 向量搜索 + 重排序 |
| **100 万条记忆** | 检索 < 1s | 分层检索 + 缓存 |
| **1000 并发提取** | 吞吐 > 100 events/s | 压力测试 |

### 基准测试

**测试数据集**：
- 100 个真实 Claude Code 会话
- 1000 条工具调用记录
- 覆盖 Rust/TypeScript/Python 项目

**测试脚本**：
```rust
#[tokio::test]
async fn benchmark_extraction() {
    let events = load_test_events(100);
    let start = Instant::now();

    let memories = batch_extract_memories(events).await.unwrap();

    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_secs(5));
    assert!(memories.len() > 50); // 至少提取 50% 有效记忆
}

#[tokio::test]
async fn benchmark_retrieval() {
    let queries = vec![
        "Rust 错误处理",
        "项目使用的数据库",
        "用户偏好的代码风格",
    ];

    for query in queries {
        let start = Instant::now();
        let results = search_memory(query, 10).await.unwrap();
        let elapsed = start.elapsed();

        assert!(elapsed < Duration::from_millis(200));
        assert!(results.len() > 0);
    }
}
```

---

## 风险与权衡

### 风险 1: LLM 提取质量不稳定

**描述**：LLM 可能提取无关信息或遗漏关键信息

**影响**：记忆质量下降，噪音增加

**缓解措施**：
1. **Prompt 工程** — 提供清晰示例和约束
2. **多轮验证** — 冲突检测阶段二次确认
3. **人工反馈** — 提供"标记为无用"功能
4. **降级策略** — LLM 失败时回退到规则提取

**权衡**：
- ✅ 接受 10% 噪音，换取 90% 自动化
- ❌ 不追求 100% 准确率（成本太高）

### 风险 2: 向量搜索召回率低

**描述**：语义搜索可能遗漏关键词匹配的记忆

**影响**：用户搜索不到明明存在的记忆

**缓解措施**：
1. **混合检索** — FTS5 + 向量搜索并行
2. **查询扩展** — 同义词、文件名提取
3. **渐进式披露** — 降低阈值重试
4. **用户反馈** — "没找到？试试这些"

**权衡**：
- ✅ 混合检索覆盖 95% 场景
- ❌ 接受 5% 边缘情况需要手动搜索

### 风险 3: 存储空间膨胀

**描述**：长期使用后数据库可能增长到 GB 级别

**影响**：检索变慢，备份困难

**缓解措施**：
1. **自动遗忘** — 定期清理低价值记忆
2. **压缩合并** — 相似记忆合并
3. **分层存储** — Archival 层可选外部存储
4. **用户控制** — 提供"清理旧记忆"工具

**权衡**：
- ✅ 100 万条记忆 = 4GB（可接受）
- ❌ 不支持无限存储（需要遗忘）

### 风险 4: 隐私泄露

**描述**：记忆中可能包含敏感信息（密钥、个人信息）

**影响**：数据泄露风险

**缓解措施**：
1. **本地优先** — 数据不离开本地
2. **敏感信息过滤** — 正则匹配密钥模式
3. **加密存储** — SQLite 数据库加密（可选）
4. **用户控制** — 提供"删除记忆"功能

**权衡**：
- ✅ 本地存储 + 加密 = 足够安全
- ❌ 不支持云同步（隐私优先）

### 风险 5: 依赖 Claude API

**描述**：网络故障或 API 限流导致提取失败

**影响**：记忆捕获中断

**缓解措施**：
1. **离线缓冲** — 事件先写入本地队列
2. **重试机制** — 指数退避重试
3. **降级策略** — API 失败时跳过提取，保留原始事件
4. **本地 LLM 备选** — 可选 Ollama 作为备用

**权衡**：
- ✅ 99% 时间 API 可用
- ❌ 接受 1% 时间记忆捕获延迟

### 权衡总结

| 维度 | 选择 | 放弃 | 理由 |
|------|------|------|------|
| **质量 vs 成本** | 质量优先 | 零成本 | 用户不在乎 $0.05/对话 |
| **自动 vs 手动** | 自动捕获 | 手动保存 | 不依赖用户自觉性 |
| **本地 vs 云端** | 本地优先 | 云同步 | 隐私保护 |
| **简单 vs 复杂** | 简单架构 | 分布式 | 易于理解和贡献 |
| **完整 vs 精简** | 完整历史 | 无限存储 | 需要遗忘机制 |

---

## 附录：竞品对比

### 功能对比矩阵

| 功能 | remem | Mem0 | Zep | Letta | Cursor | 评价 |
|------|-------|------|-----|-------|--------|------|
| **自动捕获** | ✅ Hook | ❌ 手动 | ✅ Hook | ❌ 手动 | ✅ 自动 | remem 最强 |
| **LLM 提取** | ✅ Claude | ✅ GPT-4 | ✅ 自定义 | ❌ 规则 | ❌ 规则 | remem 免费 |
| **向量搜索** | ✅ 本地 | ✅ 云端 | ✅ 云端 | ✅ 本地 | ✅ 本地 | remem 隐私 |
| **去重** | ✅ 三层 | ✅ 三层 | ✅ 两层 | ❌ 无 | ✅ Hash | remem 完整 |
| **分层存储** | ✅ 三层 | ❌ 单层 | ❌ 单层 | ✅ 三层 | ❌ 单层 | remem = Letta |
| **时间模型** | ✅ Bitemporal | ❌ 单时间 | ✅ Bitemporal | ❌ 单时间 | ❌ 单时间 | remem = Zep |
| **遗忘机制** | ✅ 自动 | ❌ 无 | ❌ 无 | ✅ 手动 | ❌ 无 | remem 最强 |
| **本地优先** | ✅ 完全 | ❌ 云端 | ❌ 云端 | ✅ 完全 | ✅ 完全 | remem = Letta |
| **成本** | $0.05/对话 | $0.20/对话 | $0.15/对话 | $0 | $0 | Letta 最低 |

### 设计理念对比

| 系统 | 核心理念 | 适用场景 | remem 借鉴 |
|------|---------|---------|-----------|
| **Mem0** | 双阶段提取 + 冲突解决 | 通用 AI 应用 | ✅ 提取管道 |
| **Zep** | 企业级 + 高并发 | 生产环境 | ✅ Bitemporal + 并发 |
| **Letta** | 三层内存 + 简单存储 | 对话 Agent | ✅ 分层架构 |
| **Cursor** | 自动索引 + 增量更新 | IDE 集成 | ✅ 增量索引 |
| **Augment** | 实时索引 + 自定义模型 | 代码搜索 | ✅ 本地 embedding |
| **LangMem** | 后台通道 + 不阻塞 | 实时交互 | ✅ 异步提取 |
| **Engram** | 渐进式披露 + UPSERT | 知识管理 | ✅ 记忆演进 |

### remem 的独特优势

1. **最强自动化** — 唯一完全自动捕获 + 自动去重 + 自动遗忘的系统
2. **最佳隐私** — 完全本地，数据不离开用户设备
3. **最低成本** — 免费 Claude API + 零基础设施成本
4. **最简架构** — 单文件数据库，易于理解和贡献
5. **最强质量** — 结合 10 个系统的最佳实践

---

## 总结

remem 的设计目标是成为 **Claude Code 的最强记忆系统**。通过深度调研 10 个竞品，我们提取了最强设计并针对开发者场景优化：

### 核心创新

1. **自动捕获是主力** — 不依赖 Claude 主动调用，PostToolUse hook 自动捕获所有工具调用
2. **质量优先于成本** — 使用 LLM 提取而非规则，准确率提升 26%
3. **本地优先架构** — SQLite + sqlite-vec，零依赖，完全隐私保护
4. **三层去重漏斗** — Hash + 向量 + LLM，降低 90% 重复
5. **Bitemporal 时间模型** — 支持时间旅行查询和审计追踪
6. **三层内存架构** — Core/Active/Archival，平衡性能和容量
7. **自动遗忘机制** — 时间衰减 + 重要性评分，防止存储膨胀

### 预期效果

- **提取延迟**：< 5s（批量 10 条事件）
- **检索延迟**：< 200ms（混合检索）
- **记忆质量**：准确率 > 90%
- **存储效率**：压缩率 > 10x
- **成本控制**：< $0.05/对话

### 实现路线

- **Phase 1 (2 周)**：MVP — 验证核心假设
- **Phase 2 (2 周)**：质量提升 — 去重 + 冲突检测
- **Phase 3 (1 周)**：性能优化 — 并行 + 缓存
- **Phase 4 (2 周)**：高级特性 — 分层 + 遗忘

### 下一步行动

1. **立即开始 Phase 1** — 实现 MVP，验证假设
2. **建立基准测试** — 100 个真实会话，量化效果
3. **用户测试** — 邀请 10 个 Claude Code 用户试用
4. **迭代优化** — 根据反馈调整设计

remem 将成为 Claude Code 用户不可或缺的记忆系统，让 AI 真正"记住"每一次对话。