# Mem0 记忆提取机制深度调研

> 调研日期：2026-03-16
> 目标：分析 Mem0 的记忆提取、存储、更新机制，为 remem 项目提供设计参考

---

## 执行摘要

Mem0 是一个生产级的 AI 记忆层，核心特点是**自动化 LLM 提取 + 智能冲突解决**。与 remem 的 zero-LLM 方向相反，Mem0 将 LLM 作为记忆质量的核心保障，通过多阶段提取管道实现高质量记忆存储。

**关键发现**：
- **每轮对话自动提取**：默认 `infer=True`，每次 `add()` 调用都触发 LLM 提取
- **双阶段提取管道**：事实提取 → 冲突解决（ADD/UPDATE/DELETE/NONE）
- **向量 + 图双存储**：向量搜索 + 知识图谱关系
- **SQLite 历史追踪**：完整的 bitemporal 历史记录
- **MD5 哈希去重**：基于内容哈希的简单去重

---

## 1. 提取触发机制

### 1.1 触发时机

```python
# 每次调用 add() 都会触发提取（如果 infer=True）
def add(self, messages, user_id=None, agent_id=None, run_id=None,
        metadata=None, infer=True, memory_type=None, prompt=None):
```

**触发条件**：
- **同步触发**：`add()` 调用时立即执行，阻塞返回
- **默认开启**：`infer=True` 是默认值
- **可选关闭**：`infer=False` 时直接存储原始消息，跳过提取

**并发处理**：
```python
# 向量存储和图存储并行执行
with concurrent.futures.ThreadPoolExecutor() as executor:
    future1 = executor.submit(self._add_to_vector_store, messages, metadata, filters, infer)
    future2 = executor.submit(self._add_to_graph, messages, filters)
    concurrent.futures.wait([future1, future2])
```

### 1.2 提取管道架构

```
┌─────────────────────────────────────────────────────────────┐
│                    add(messages, user_id)                    │
└────────────────────────────┬────────────────────────────────┘
                             │
                ┌────────────┴────────────┐
                │                         │
        ┌───────▼────────┐       ┌───────▼────────┐
        │ Vector Store   │       │  Graph Store   │
        │   Pipeline     │       │   Pipeline     │
        └───────┬────────┘       └───────┬────────┘
                │                         │
                │                         │
    ┌───────────▼───────────┐             │
    │ 1. Parse Messages     │             │
    │    (parse_messages)   │             │
    └───────────┬───────────┘             │
                │                         │
    ┌───────────▼───────────┐             │
    │ 2. Fact Extraction    │             │
    │    (LLM Call #1)      │             │
    │    → facts: []        │             │
    └───────────┬───────────┘             │
                │                         │
    ┌───────────▼───────────┐             │
    │ 3. Embed Facts        │             │
    │    (Embedding Model)  │             │
    └───────────┬───────────┘             │
                │                         │
    ┌───────────▼───────────┐             │
    │ 4. Search Existing    │             │
    │    (Vector Search)    │             │
    │    → old_memories     │             │
    └───────────┬───────────┘             │
                │                         │
    ┌───────────▼───────────┐             │
    │ 5. Conflict Resolution│             │
    │    (LLM Call #2)      │             │
    │    → ADD/UPDATE/DELETE│             │
    └───────────┬───────────┘             │
                │                         │
    ┌───────────▼───────────┐             │
    │ 6. Execute Actions    │             │
    │    (Vector Store CRUD)│             │
    └───────────┬───────────┘             │
                │                         │
    ┌───────────▼───────────┐    ┌────────▼────────┐
    │ 7. History Logging    │    │ Entity Extraction│
    │    (SQLite)           │    │ (Graph Store)    │
    └───────────────────────┘    └─────────────────┘
```

---

## 2. 提取内容与粒度

### 2.1 记忆类型分类

Mem0 区分两种提取模式：

#### User Memory Extraction（用户记忆）
```python
# 只提取用户消息中的信息
USER_MEMORY_EXTRACTION_PROMPT = """
# [IMPORTANT]: GENERATE FACTS SOLELY BASED ON THE USER'S MESSAGES.
# DO NOT INCLUDE INFORMATION FROM ASSISTANT OR SYSTEM MESSAGES.

Types of Information to Remember:
1. Store Personal Preferences
2. Maintain Important Personal Details
3. Track Plans and Intentions
4. Remember Activity and Service Preferences
5. Monitor Health and Wellness Preferences
6. Store Professional Details
7. Miscellaneous Information Management
"""
```

**示例**：
```
Input:
user: Hi, my name is John. I am a software engineer.
assistant: Nice to meet you, John! My name is Alex.

Output:
{"facts": ["Name is John", "Is a Software engineer"]}
```

#### Agent Memory Extraction（助手记忆）
```python
# 只提取助手消息中的信息
AGENT_MEMORY_EXTRACTION_PROMPT = """
# [IMPORTANT]: GENERATE FACTS SOLELY BASED ON THE ASSISTANT'S MESSAGES.

Types of Information to Remember:
1. Assistant's Preferences
2. Assistant's Capabilities
3. Assistant's Hypothetical Plans or Activities
4. Assistant's Personality Traits
5. Assistant's Approach to Tasks
6. Assistant's Knowledge Areas
"""
```

**触发条件**：
```python
def _should_use_agent_memory_extraction(self, messages, metadata):
    has_agent_id = metadata.get("agent_id") is not None
    has_assistant_messages = any(msg.get("role") == "assistant" for msg in messages)
    return has_agent_id and has_assistant_messages
```

### 2.2 提取粒度

**单条事实（Atomic Facts）**：
```json
{
  "facts": [
    "Name is John",
    "Is a Software engineer",
    "Favourite movies are Inception and Interstellar"
  ]
}
```

**粒度控制原则**：
- 每个事实是独立的、可验证的陈述
- 避免复合句（"John is a software engineer and likes pizza" → 拆分为两条）
- 保持语言一致性（用户用中文 → 事实用中文记录）

### 2.3 过滤规则

**不提取的内容**：
```python
# 1. 系统消息
if msg["role"] == "system":
    continue

# 2. 空消息或无意义对话
Input: "Hi."
Output: {"facts": []}

Input: "There are branches in trees."  # 常识，非个人信息
Output: {"facts": []}
```

**元数据提取**：
```python
# 自动提取的元数据
metadata = {
    "data": fact_text,
    "hash": hashlib.md5(fact_text.encode()).hexdigest(),
    "created_at": datetime.now(pytz.timezone("US/Pacific")).isoformat(),
    "user_id": user_id,
    "agent_id": agent_id,
    "run_id": run_id,
    "actor_id": message.get("name"),  # 从消息中提取
    "role": message.get("role")
}
```

---

## 3. 存储机制

### 3.1 数据模型

#### 向量存储（Vector Store）
```python
# 存储结构
{
    "id": "uuid",
    "vector": [0.1, 0.2, ...],  # 嵌入向量
    "payload": {
        "data": "User likes sci-fi movies",
        "hash": "a1b2c3d4...",
        "created_at": "2026-03-16T10:00:00-08:00",
        "updated_at": "2026-03-16T10:00:00-08:00",
        "user_id": "alice",
        "agent_id": null,
        "run_id": null,
        "actor_id": "alice",
        "role": "user"
    }
}
```

#### 历史数据库（SQLite）
```sql
CREATE TABLE history (
    id           TEXT PRIMARY KEY,
    memory_id    TEXT,
    old_memory   TEXT,
    new_memory   TEXT,
    event        TEXT,  -- ADD/UPDATE/DELETE
    created_at   DATETIME,
    updated_at   DATETIME,
    is_deleted   INTEGER,
    actor_id     TEXT,
    role         TEXT
)
```

**Bitemporal 特性**：
- `created_at`：记忆首次创建时间（不变）
- `updated_at`：最后修改时间
- 历史表保留所有版本，支持完整审计

### 3.2 存储后端

**支持的向量数据库**（28+ 种）：
- Qdrant（默认）
- Pinecone
- Chroma
- Weaviate
- Milvus
- Elasticsearch
- PostgreSQL (pgvector)
- Azure AI Search
- ...

**图数据库**（可选）：
- Neo4j
- Memgraph
- Kuzu
- Neptune

### 3.3 去重策略

#### 基于哈希的简单去重
```python
# 创建记忆时计算哈希
metadata["hash"] = hashlib.md5(data.encode()).hexdigest()
```

**局限性**：
- 只能检测完全相同的文本
- 无法识别语义相似但表述不同的记忆
- 依赖后续的冲突解决阶段

#### 向量搜索去重
```python
# 对每个新事实，搜索相似的旧记忆
for new_mem in new_retrieved_facts:
    messages_embeddings = self.embedding_model.embed(new_mem, "add")
    existing_memories = self.vector_store.search(
        query=new_mem,
        vectors=messages_embeddings,
        limit=5,  # 只检索 Top-5
        filters=search_filters,
    )
```

**去重逻辑**：
- 搜索 Top-5 相似记忆
- 交给 LLM 判断是否重复/冲突/需要更新
- 不是硬性去重，而是智能合并

---

## 4. 更新机制

### 4.1 冲突检测

**两阶段 LLM 调用**：

#### 阶段 1：事实提取
```python
response = self.llm.generate_response(
    messages=[
        {"role": "system", "content": FACT_RETRIEVAL_PROMPT},
        {"role": "user", "content": f"Input:\n{parsed_messages}"},
    ],
    response_format={"type": "json_object"},
)
# → {"facts": ["New fact 1", "New fact 2"]}
```

#### 阶段 2：冲突解决
```python
# 构造 prompt
function_calling_prompt = get_update_memory_messages(
    retrieved_old_memory_dict,  # 旧记忆
    new_retrieved_facts,        # 新事实
    custom_update_memory_prompt
)

response = self.llm.generate_response(
    messages=[{"role": "user", "content": function_calling_prompt}],
    response_format={"type": "json_object"},
)
# → {"memory": [{"id": "0", "text": "...", "event": "UPDATE", "old_memory": "..."}]}
```

### 4.2 更新决策逻辑

**四种操作**：

#### 1. ADD（新增）
```python
# 条件：新信息不存在于旧记忆中
{
    "id": "1",  # 新 ID
    "text": "Name is John",
    "event": "ADD"
}
```

#### 2. UPDATE（更新）
```python
# 条件：信息有更多细节或矛盾
# 示例：
# 旧："User likes to play cricket"
# 新："Loves to play cricket with friends"
# → UPDATE（更详细）

{
    "id": "2",  # 保持旧 ID
    "text": "Loves to play cricket with friends",
    "event": "UPDATE",
    "old_memory": "User likes to play cricket"
}
```

**更新规则**：
```python
# Prompt 中的指导
"""
If the retrieved facts contain information that is already present in the memory
but the information is totally different, then you have to update it.

If the retrieved fact contains information that conveys the same thing as the
elements present in the memory, then you have to keep the fact which has the
most information.

Example (a) -- if the memory contains "User likes to play cricket" and the
retrieved fact is "Loves to play cricket with friends", then update the memory.

Example (b) -- if the memory contains "Likes cheese pizza" and the retrieved
fact is "Loves cheese pizza", then you do not need to update it because they
convey the same information.
"""
```

#### 3. DELETE（删除）
```python
# 条件：新信息与旧记忆矛盾
# 示例：
# 旧："Loves cheese pizza"
# 新："Dislikes cheese pizza"
# → DELETE

{
    "id": "1",
    "text": "Loves cheese pizza",
    "event": "DELETE"
}
```

#### 4. NONE（无操作）
```python
# 条件：信息已存在且无需更新
{
    "id": "0",
    "text": "Name is John",
    "event": "NONE"
}
```

**特殊处理**：
```python
# NONE 事件仍会更新 session IDs
elif event_type == "NONE":
    memory_id = temp_uuid_mapping.get(resp.get("id"))
    if memory_id and (metadata.get("agent_id") or metadata.get("run_id")):
        # 更新 agent_id/run_id，保持内容不变
        updated_metadata = deepcopy(existing_memory.payload)
        if metadata.get("agent_id"):
            updated_metadata["agent_id"] = metadata["agent_id"]
        if metadata.get("run_id"):
            updated_metadata["run_id"] = metadata["run_id"]
        updated_metadata["updated_at"] = datetime.now().isoformat()

        self.vector_store.update(
            vector_id=memory_id,
            vector=None,  # 保持原向量
            payload=updated_metadata,
        )
```

### 4.3 UUID 幻觉防护

**问题**：LLM 可能生成不存在的 UUID

**解决方案**：
```python
# 映射 UUID 到整数
temp_uuid_mapping = {}
for idx, item in enumerate(retrieved_old_memory):
    temp_uuid_mapping[str(idx)] = item["id"]
    retrieved_old_memory[idx]["id"] = str(idx)

# LLM 返回整数 ID
# 执行时映射回真实 UUID
real_uuid = temp_uuid_mapping[resp.get("id")]
```

### 4.4 时间感知

**时间戳管理**：
```python
# 创建时
metadata["created_at"] = datetime.now(pytz.timezone("US/Pacific")).isoformat()

# 更新时
new_metadata["created_at"] = existing_memory.payload.get("created_at")  # 保持不变
new_metadata["updated_at"] = datetime.now(pytz.timezone("US/Pacific")).isoformat()
```

**自定义时间戳**（Platform 功能）：
```python
# 支持导入历史数据
client.add(messages, user_id="alice", timestamp=1672531200)  # Unix timestamp
```

**无衰减机制**：
- Mem0 不实现自动衰减
- 依赖显式删除或过期时间（Platform 功能）
- 访问频率不影响记忆权重

---

## 5. 架构设计亮点

### 5.1 并行执行

```python
# 向量和图存储并行
with concurrent.futures.ThreadPoolExecutor() as executor:
    future1 = executor.submit(self._add_to_vector_store, ...)
    future2 = executor.submit(self._add_to_graph, ...)
    concurrent.futures.wait([future1, future2])
```

### 5.2 工厂模式

```python
# 支持多种后端的统一接口
self.embedding_model = EmbedderFactory.create(
    self.config.embedder.provider,
    self.config.embedder.config,
)
self.vector_store = VectorStoreFactory.create(
    self.config.vector_store.provider,
    self.config.vector_store.config,
)
self.llm = LlmFactory.create(
    self.config.llm.provider,
    self.config.llm.config,
)
```

### 5.3 Reranker 支持

```python
# 可选的重排序
if rerank and self.reranker and original_memories:
    try:
        reranked_memories = self.reranker.rerank(query, original_memories, limit)
        original_memories = reranked_memories
    except Exception as e:
        logger.warning(f"Reranking failed, using original results: {e}")
```

### 5.4 元数据过滤增强

```python
# 支持复杂查询
filters = {
    "AND": [
        {"category": "movie"},
        {"rating": {"gte": 4.0}}
    ],
    "OR": [
        {"genre": "sci-fi"},
        {"genre": "thriller"}
    ]
}
```

---

## 6. 代码统计

| 文件 | 行数 | 职责 |
|------|------|------|
| `mem0/memory/main.py` | 2325 | 核心记忆管理逻辑 |
| `mem0/configs/prompts.py` | 459 | 提取和更新 prompt |
| `mem0/memory/utils.py` | 208 | 工具函数 |
| `mem0/memory/storage.py` | 218 | SQLite 历史管理 |
| **总计** | **3210** | |

**关键函数**：
- `add()`: 281 行起，主入口
- `_add_to_vector_store()`: 386 行起，提取管道
- `_create_memory()`: 1075 行起，创建记忆
- `_update_memory()`: 1142 行起，更新记忆
- `_delete_memory()`: 1196 行起，删除记忆

---

## 7. 与 remem 的对比

| 维度 | Mem0 | remem (当前) | 建议 |
|------|------|--------------|------|
| **提取触发** | 每轮自动（LLM） | 无自动提取 | ❌ 必须恢复自动提取 |
| **提取质量** | 双阶段 LLM | N/A | ✅ 借鉴双阶段设计 |
| **冲突解决** | LLM 智能判断 | 无 | ✅ 实现冲突检测 |
| **去重策略** | 哈希 + 向量搜索 | 无 | ✅ 实现向量去重 |
| **历史追踪** | SQLite bitemporal | 无 | ✅ 添加历史表 |
| **存储后端** | 28+ 向量 DB | SQLite only | ⚠️ 先做好 SQLite |
| **图存储** | 可选 Neo4j 等 | 无 | ⏸️ 暂不需要 |
| **成本** | 每轮 2 次 LLM 调用 | 0 | ⚠️ 可接受（质量优先）|

---

## 8. 关键教训

### 8.1 不要砍掉 LLM 提取

**Mem0 的成功证明**：
- 26% 准确率提升（vs OpenAI Memory）
- 91% 更快响应
- 90% 更少 token

**原因**：
- LLM 提取 ≠ 全量上下文
- 只提取关键事实，大幅减少存储和检索成本
- 智能冲突解决避免记忆膨胀

### 8.2 双阶段提取是必要的

**阶段 1（事实提取）**：
- 输入：原始对话
- 输出：结构化事实列表
- 作用：降噪、标准化

**阶段 2（冲突解决）**：
- 输入：新事实 + 旧记忆
- 输出：ADD/UPDATE/DELETE 决策
- 作用：去重、合并、更新

**不能合并的原因**：
- 第一阶段需要完整对话上下文
- 第二阶段需要检索结果
- 分离后可以缓存提取结果

### 8.3 历史追踪是生产必需

```sql
-- Mem0 的历史表设计值得借鉴
CREATE TABLE history (
    id           TEXT PRIMARY KEY,
    memory_id    TEXT,           -- 关联到向量存储的 ID
    old_memory   TEXT,           -- 旧值
    new_memory   TEXT,           -- 新值
    event        TEXT,           -- ADD/UPDATE/DELETE
    created_at   DATETIME,       -- 操作时间
    updated_at   DATETIME,
    is_deleted   INTEGER,
    actor_id     TEXT,           -- 谁触发的
    role         TEXT            -- user/assistant
)
```

**用途**：
- 审计：谁在什么时候修改了什么
- 回滚：恢复到历史版本
- 分析：记忆演化趋势

### 8.4 元数据设计要前瞻

**Mem0 的元数据字段**：
```python
{
    "data": str,           # 记忆内容
    "hash": str,           # MD5 哈希
    "created_at": str,     # ISO 8601
    "updated_at": str,
    "user_id": str,        # 用户标识
    "agent_id": str,       # 助手标识
    "run_id": str,         # 会话标识
    "actor_id": str,       # 消息发送者
    "role": str,           # user/assistant/system
    # + 自定义字段
}
```

**设计原则**：
- 核心字段提升到顶层（便于过滤）
- 自定义字段放 `metadata` 子对象
- 时间字段用 ISO 8601（带时区）
- 保留扩展性（不要硬编码字段）

---

## 9. 实现建议

### 9.1 短期（1-2 周）

1. **恢复 LLM 提取**
   ```rust
   // src/extraction/mod.rs
   pub async fn extract_facts(messages: &[Message]) -> Result<Vec<Fact>> {
       // 调用 LLM 提取事实
   }
   ```

2. **实现双阶段管道**
   ```rust
   // src/memory/pipeline.rs
   pub async fn add_memory(messages: &[Message]) -> Result<Vec<MemoryOp>> {
       let facts = extract_facts(messages).await?;
       let old_memories = search_similar(&facts).await?;
       let ops = resolve_conflicts(&facts, &old_memories).await?;
       execute_ops(&ops).await?;
       Ok(ops)
   }
   ```

3. **添加历史表**
   ```sql
   CREATE TABLE memory_history (
       id INTEGER PRIMARY KEY,
       memory_id TEXT NOT NULL,
       old_value TEXT,
       new_value TEXT,
       operation TEXT NOT NULL,  -- ADD/UPDATE/DELETE
       created_at TEXT NOT NULL,
       actor TEXT
   );
   ```

### 9.2 中期（1 个月）

1. **向量去重**
   ```rust
   // 搜索相似记忆
   let similar = vector_store.search(&embedding, limit=5).await?;

   // 交给 LLM 判断
   let decision = llm.resolve_conflict(&new_fact, &similar).await?;
   ```

2. **元数据标准化**
   ```rust
   pub struct MemoryMetadata {
       pub user_id: Option<String>,
       pub session_id: Option<String>,
       pub actor: Option<String>,
       pub role: MessageRole,
       pub created_at: DateTime<Utc>,
       pub updated_at: DateTime<Utc>,
       pub custom: HashMap<String, Value>,
   }
   ```

3. **Prompt 工程**
   - 借鉴 Mem0 的 few-shot examples
   - 添加中文支持
   - 针对 Claude Code 场景优化

### 9.3 长期（3 个月）

1. **多后端支持**
   - 保持 SQLite 作为默认
   - 添加 Qdrant/Chroma 可选支持
   - 统一接口（类似 Mem0 的 Factory）

2. **Reranker 集成**
   - 向量搜索 → Reranker → 最终结果
   - 提升检索精度

3. **图存储（可选）**
   - 仅在需要关系推理时启用
   - 不作为核心依赖

---

## 10. 参考资源

### 代码位置
- **核心逻辑**：`mem0/memory/main.py`
- **Prompt**：`mem0/configs/prompts.py`
- **工具函数**：`mem0/memory/utils.py`
- **历史管理**：`mem0/memory/storage.py`

### 文档
- 官方文档：https://docs.mem0.ai
- 论文：https://mem0.ai/research
- GitHub：https://github.com/mem0ai/mem0

### 关键 Prompt

#### 事实提取 Prompt
```
You are a Personal Information Organizer, specialized in accurately storing
facts, user memories, and preferences. Your primary role is to extract relevant
pieces of information from conversations and organize them into distinct,
manageable facts.

Types of Information to Remember:
1. Store Personal Preferences
2. Maintain Important Personal Details
3. Track Plans and Intentions
4. Remember Activity and Service Preferences
5. Monitor Health and Wellness Preferences
6. Store Professional Details
7. Miscellaneous Information Management

Return the facts in JSON format: {"facts": ["fact1", "fact2"]}
```

#### 冲突解决 Prompt
```
You are a smart memory manager which controls the memory of a system.
You can perform four operations: (1) add, (2) update, (3) delete, (4) no change.

Compare newly retrieved facts with the existing memory. For each new fact, decide:
- ADD: New information not present in memory
- UPDATE: Information exists but needs more detail or correction
- DELETE: New information contradicts existing memory
- NONE: Information already present and accurate

Return format:
{
  "memory": [
    {
      "id": "0",
      "text": "Updated memory text",
      "event": "UPDATE",
      "old_memory": "Previous text"
    }
  ]
}
```

---

## 11. 结论

Mem0 的成功证明了**自动化 LLM 提取是高质量记忆系统的核心**。remem 的 zero-LLM 方向是错误的——不是因为成本，而是因为质量。

**核心要点**：
1. ✅ **必须恢复 LLM 提取**：这是记忆质量的基础
2. ✅ **双阶段管道**：事实提取 + 冲突解决
3. ✅ **历史追踪**：SQLite bitemporal 表
4. ✅ **向量去重**：基于语义相似度
5. ⚠️ **成本可控**：只提取关键事实，不是全量上下文

**下一步行动**：
1. 立即恢复 `extract_facts()` 函数
2. 实现双阶段提取管道
3. 添加 `memory_history` 表
4. 编写 Mem0 风格的 prompt
5. 测试并迭代

**记住**：remem 的目标是**最强**的记忆系统，不是最便宜的。Mem0 已经证明了正确的方向。
