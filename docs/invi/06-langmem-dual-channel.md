# LangMem 双通道记忆机制深度调研

> 调研日期：2026-03-16
> 目标：理解 LangMem 的双通道记忆提取、存储、更新机制，为 remem 设计提供参考

---

## 1. 核心架构

### 1.1 设计理念

LangMem 的核心理念是**让 Agent 从交互中学习**，而不是依赖离线数据摄入。所有记忆操作遵循统一模式：

```
输入：对话 + 当前记忆状态
处理：LLM 分析如何扩展/合并记忆
输出：更新后的记忆状态
```

**关键设计决策**：
- **质量优先**：不惜成本提升记忆质量（与 remem 目标一致）
- **LLM 驱动**：所有记忆提取/更新都通过 LLM 完成，不依赖规则
- **模块化**：核心 API 无状态，存储层可插拔
- **类型化**：通过 Pydantic schema 强制记忆结构

---

## 2. 双通道机制

### 2.1 通道对比

| 维度 | Conscious (Hot Path) | Subconscious (Background) |
|------|---------------------|---------------------------|
| **触发时机** | 对话进行中 | 对话结束后 / 空闲时 |
| **延迟影响** | 增加响应延迟 | 无延迟影响 |
| **更新速度** | 立即生效 | 延迟生效 |
| **适用场景** | 关键上下文更新 | 模式分析、摘要 |
| **实现方式** | Agent 主动调用工具 | 后台 Reflection Executor |

### 2.2 Conscious 通道（实时）

**实现方式**：Agent 在对话中主动调用 `manage_memory` 工具

```python
# 创建记忆管理工具
memory_tool = create_manage_memory_tool(
    namespace=("memories", "{langgraph_user_id}"),
    instructions="""Proactively call this tool when you:
    1. Identify a new USER preference
    2. Receive explicit request to remember something
    3. Want to record important context
    4. Identify existing MEMORY is incorrect/outdated
    """
)

# Agent 在对话中调用
agent = create_react_agent(
    "anthropic:claude-3-5-sonnet-latest",
    tools=[memory_tool],
    store=store
)
```

**工具签名**：
```python
def manage_memory(
    content: str | Schema,      # 记忆内容
    action: Literal["create", "update", "delete"],
    id: UUID | None = None      # update/delete 时必填
) -> str
```

**问题**：依赖 Agent 的"自觉性"，Claude Code 实际上不会主动调用（remem 的错误 2）

### 2.3 Subconscious 通道（后台）

**实现方式**：通过 `ReflectionExecutor` 在后台异步处理

```python
# 创建后台记忆管理器
manager = create_memory_store_manager(
    "anthropic:claude-3-5-sonnet-latest",
    namespace=("memories", "{langgraph_user_id}"),
    store=store
)

# 创建后台执行器
executor = ReflectionExecutor(
    reflector=manager,  # 或远程 graph 名称
    store=store
)

# 在对话节点中提交后台任务
def chat_node(state):
    # ... 正常对话逻辑 ...

    # 提交后台记忆提取（不阻塞）
    executor.submit(
        {"messages": state["messages"]},
        after_seconds=5  # 延迟 5 秒执行
    )
    return response
```

**ReflectionExecutor 特性**：
- **任务队列**：使用 `PriorityQueue` 管理待执行任务
- **去重机制**：同一 thread_id 的新任务会取消旧任务
- **延迟执行**：`after_seconds` 参数控制延迟
- **取消支持**：通过 `cancel_event` 取消过时任务

**本地 vs 远程**：
```python
# 本地执行（同进程）
LocalReflectionExecutor(reflector=manager, store=store)

# 远程执行（LangGraph Cloud）
RemoteReflectionExecutor(
    namespace=("memories", "user_id"),
    reflector="memory_graph_name",  # 远程 graph
    url="https://api.langchain.com"
)
```

---

## 3. 记忆类型

### 3.1 Semantic Memory（语义记忆）

**用途**：存储事实、知识、偏好

**两种模式**：

#### Collection（集合）
- 无界文档集合，运行时搜索
- 支持 insert/update/delete
- 需要平衡创建与合并（precision vs recall）

```python
manager = create_memory_manager(
    "anthropic:claude-3-5-sonnet-latest",
    enable_inserts=True,
    enable_updates=True,
    enable_deletes=True
)
```

#### Profile（档案）
- 单一文档，严格 schema
- 只更新不创建新文档
- 适合用户偏好、当前状态

```python
class UserProfile(BaseModel):
    name: str
    preferred_name: str
    response_style: str
    skills: list[str]

manager = create_memory_manager(
    model,
    schemas=[UserProfile],
    enable_inserts=False  # 只更新
)
```

### 3.2 Episodic Memory（情节记忆）

**用途**：捕获成功交互的完整推理链

**Schema 设计**：
```python
class Episode(BaseModel):
    observation: str  # 情境和上下文
    thoughts: str     # 内部推理过程
    action: str       # 采取的行动
    result: str       # 结果和为何成功
```

**与 Semantic 的区别**：
- Semantic：存储"是什么"（Python 是编程语言）
- Episodic：存储"怎么做"（用食谱类比解释 Python 比用蛇类比更有效）

### 3.3 Procedural Memory（程序记忆）

**用途**：编码 Agent 行为规则，通过反馈优化 system prompt

**实现**：Prompt Optimization

```python
optimizer = create_prompt_optimizer(
    "anthropic:claude-3-5-sonnet-latest",
    kind="gradient",  # 或 "metaprompt" / "prompt_memory"
    config={"max_reflection_steps": 3}
)

optimized_prompt = optimizer.invoke({
    "trajectories": [(messages, {"user_score": 0})],
    "prompt": current_prompt
})
```

---

## 4. 提取机制

### 4.1 MemoryManager（核心提取器）

**工作流程**：

```python
class MemoryManager:
    def ainvoke(self, input: MemoryState) -> list[ExtractedMemory]:
        # 1. 准备消息
        prepared_messages = self._prepare_messages(
            input["messages"],
            max_steps
        )

        # 2. 准备现有记忆
        prepared_existing = self._prepare_existing(input.get("existing"))

        # 3. 创建提取器（使用 trustcall）
        extractor = create_extractor(
            self.model,
            tools=list(self.schemas),  # Memory schemas 作为工具
            enable_inserts=True,
            enable_updates=True,
            enable_deletes=True
        )

        # 4. 多步提取循环
        results = {}
        for i in range(max_steps):
            if i == 1:
                # 第二步加入 Done 工具
                extractor = create_extractor(..., tools=[...schemas, Done])

            response = await extractor.ainvoke({
                "messages": prepared_messages,
                "existing": prepared_existing
            })

            # 5. 处理响应（insert/update/delete）
            for r, rmeta in zip(response["responses"], response["response_metadata"]):
                if r.__repr_name__() == "Done":
                    is_done = True
                    continue
                mem_id = rmeta.get("json_doc_id", str(uuid.uuid4()))
                results[mem_id] = r

            if is_done or not response["messages"][-1].tool_calls:
                break

        return self._filter_response(results, external_ids)
```

**关键技术**：
- **Parallel Tool Calling**：一次 LLM 调用处理多个记忆操作
- **Multi-step Refinement**：允许多轮迭代合并记忆
- **Done Signal**：Agent 自主决定何时完成

### 4.2 Prompt 设计

**系统 Prompt**（`_MEMORY_INSTRUCTIONS`）：

```
You are a long-term memory manager maintaining semantic, procedural, and episodic memory.

1. Extract & Contextualize
   - Identify essential facts, relationships, preferences, reasoning procedures
   - Caveat uncertain information with confidence levels p(x)
   - Quote supporting information when necessary

2. Compare & Update
   - Attend to novel information deviating from existing memories
   - Consolidate redundant memories; maximize SNR
   - Remove incorrect/redundant memories

3. Synthesize & Reason
   - What can you conclude about user/agent/environment?
   - What patterns, relationships, principles emerge?
   - Qualify conclusions with probabilistic confidence

Prioritize retention of:
- Surprising information (pattern deviation)
- Persistent information (frequently reinforced)
```

**用户 Prompt**：
```xml
<session_{uuid}>
{conversation}
</session_{uuid}>

Enrich, prune, and organize memories based on new information.
If existing memory is incorrect/outdated, update it.
All operations must be done in single parallel multi-tool call.
```

### 4.3 冲突解决策略

**更新逻辑**：
```python
# 1. 搜索相关记忆（语义搜索 + 时间窗口）
store_map = self._sort_results(search_results, query_limit)

# 2. LLM 决定如何处理
enriched = await memory_manager.ainvoke({
    "messages": messages,
    "existing": store_based  # 传入现有记忆
})

# 3. 应用变更
for extracted in enriched:
    if extracted.content.__repr_name__() == "RemoveDoc":
        # 删除
        removed_ids.append(extracted.id)
    elif extracted.id in store_dict:
        # 更新
        store_dict[extracted.id] = extracted.content
    else:
        # 插入
        ephemeral_dict[extracted.id] = extracted.content
```

**RemoveDoc 机制**：
```python
class RemoveDoc(BaseModel):
    json_doc_id: str  # 要删除的记忆 ID
```

LLM 通过调用 `RemoveDoc` 工具显式删除过时记忆。

---

## 5. 存储架构

### 5.1 BaseStore 接口

LangMem 依赖 LangGraph 的 `BaseStore` 接口：

```python
class BaseStore(Protocol):
    def put(self, namespace: tuple[str, ...], key: str, value: dict):
        """存储记忆"""

    def get(self, namespace: tuple[str, ...], key: str) -> Item | None:
        """获取记忆"""

    def search(
        self,
        namespace: tuple[str, ...],
        query: str | None = None,
        filter: dict | None = None,
        limit: int = 10
    ) -> list[Item]:
        """搜索记忆（语义 + 元数据过滤）"""

    def delete(self, namespace: tuple[str, ...], key: str):
        """删除记忆"""
```

**实现**：
- `InMemoryStore`：内存存储（开发用）
- `AsyncPostgresStore`：PostgreSQL + pgvector

### 5.2 Namespace 组织

**层级结构**：
```python
namespace = ("organization", "{user_id}", "context")
# 例如：("acme_corp", "user-123", "code_assistant")
```

**动态模板**：
```python
class NamespaceTemplate:
    def __init__(self, namespace: tuple[str, ...]):
        self.template = namespace

    def __call__(self, config: RunnableConfig | None = None) -> tuple[str, ...]:
        # 从 config["configurable"] 填充模板变量
        return tuple(
            config["configurable"][var] if "{" in part else part
            for part in self.template
        )
```

### 5.3 记忆结构

**存储格式**：
```python
{
    "namespace": ["memories", "user-123"],
    "key": "preference-001",
    "value": {
        "kind": "UserProfile",  # Schema 名称
        "content": {            # Schema 序列化
            "name": "Alex",
            "preferred_name": "Lex",
            "response_style": "casual"
        }
    },
    "created_at": "2025-02-07T01:12:14Z",
    "updated_at": "2025-02-07T01:12:14Z"
}
```

---

## 6. Prompt Optimization（Prompt 梯度优化）

### 6.1 三种算法

#### Gradient Optimizer（2-10 LLM 调用）
```python
# 1. Reflection 阶段（2-5 步）
tools = [think, critique, recommend]
for step in range(max_reflection_steps):
    response = await chain.ainvoke(messages)
    if response.warrants_adjustment:
        break

# 2. Update 阶段（1 步）
improved_prompt = await update_prompt(
    hypotheses=response.hypotheses,
    recommendations=response.full_recommendations,
    current_prompt=prompt
)
```

**Reflection Prompt**：
```
Analyze trajectories and evaluate:
1. How effectively assistant fulfilled user's intent
2. Where assistant deviated from expectations
3. Specific areas needing improvement

Identify failure modes:
- Style mismatch
- Unclear/incomplete instructions
- Flawed logic/reasoning
- Hallucination

Recommend minimal required changes to fix the problem.
```

**Update Metaprompt**：
```
Current prompt: {current_prompt}
Hypotheses: {hypotheses}
Recommendations: {recommendations}

Respond with updated prompt. ONLY make clearly necessary changes.
Aim to be minimally invasive.
```

#### Metaprompt Optimizer（1-5 LLM 调用）
直接应用 meta-learning，无需分离 reflection 和 update。

#### Prompt Memory Optimizer（1 LLM 调用）
最轻量级，单步优化。

### 6.2 Trajectory 格式

```python
AnnotatedTrajectory = tuple[
    list[AnyMessage],           # 对话消息
    dict[str, Any] | None       # 反馈（可选）
]

# 示例
trajectory = (
    [
        {"role": "user", "content": "Explain inheritance"},
        {"role": "assistant", "content": "Theoretical explanation..."},
        {"role": "user", "content": "Show practical example instead"}
    ],
    {"user_score": 0, "comment": "Too theoretical"}
)
```

---

## 7. MemoryStoreManager（有状态集成）

### 7.1 完整工作流

```python
class MemoryStoreManager:
    async def ainvoke(self, input: MemoryStoreManagerInput):
        # 1. 搜索相关记忆
        if self.query_gen:
            # 使用 LLM 生成搜索查询
            query_req = await self.query_gen.ainvoke(
                f"Search for memories relevant to: {conversation}"
            )
            search_results = await asyncio.gather(*[
                store.asearch(namespace, **tc["args"])
                for tc in query_req.tool_calls
            ])
        else:
            # 使用时间窗口搜索
            queries = get_dialated_windows(messages, query_limit // 4)
            search_results = await asyncio.gather(*[
                store.asearch(namespace, query=q)
                for q in queries
            ])

        # 2. 排序并限制数量
        store_map = self._sort_results(search_results, query_limit)

        # 3. 提取/更新记忆
        enriched = await self.memory_manager.ainvoke({
            "messages": input["messages"],
            "existing": [(id, item.value["kind"], item.value["content"])
                         for id, item in store_map.items()]
        })

        # 4. 应用变更
        store_based, ephemeral, removed = self._apply_manager_output(
            enriched, store_based, store_map, ephemeral
        )

        # 5. 多阶段处理（可选）
        for phase in self.phases:
            phase_manager = self._build_phase_manager(phase)
            phase_enriched = await phase_manager.ainvoke(...)
            # 再次应用变更

        # 6. 持久化
        await asyncio.gather(
            *(store.aput(**put) for put in final_puts),
            *(store.adelete(ns, key) for (ns, key) in final_deletes)
        )
```

### 7.2 搜索策略

**LLM 生成查询**（推荐）：
```python
query_gen = model.bind_tools([search_memory_tool], tool_choice="any")
query_req = await query_gen.ainvoke(
    "Use parallel tool calling to search for distinct memories relevant to this conversation"
)
# LLM 返回多个 search_memory 调用，每个查询不同方面
```

**时间窗口搜索**（fallback）：
```python
def get_dialated_windows(messages, num_windows):
    # 从最近消息开始，逐步扩大时间窗口
    # 例如：最近 1 条、最近 3 条、最近 10 条...
    return [
        " ".join(msg["content"] for msg in messages[-window_size:])
        for window_size in [1, 3, 10, 30, 100][:num_windows]
    ]
```

### 7.3 Phases（多阶段处理）

```python
manager = create_memory_store_manager(
    model,
    phases=[
        {
            "instructions": "Deduplicate and consolidate memories",
            "include_messages": False,  # 不包含原始对话
            "enable_inserts": False,
            "enable_deletes": True
        },
        {
            "instructions": "Extract high-level patterns",
            "include_messages": True,
            "enable_inserts": True,
            "enable_deletes": False
        }
    ]
)
```

**用途**：
1. 第一阶段：去重合并
2. 第二阶段：提取模式
3. 第三阶段：生成摘要

---

## 8. 关键实现细节

### 8.1 Trustcall 集成

LangMem 使用 `trustcall` 库实现结构化提取：

```python
from trustcall import create_extractor

extractor = create_extractor(
    model,
    tools=[Memory, UserProfile, Episode],  # Pydantic schemas 作为工具
    enable_inserts=True,
    enable_updates=True,
    enable_deletes=True,
    existing_schema_policy=False  # 允许修改现有记忆的 schema
)

response = await extractor.ainvoke({
    "messages": [...],
    "existing": [
        ("id-1", "Memory", Memory(content="...")),
        ("id-2", "UserProfile", UserProfile(...))
    ]
})

# response["responses"] 包含提取的记忆对象
# response["response_metadata"] 包含元数据（如 json_doc_id）
```

### 8.2 ID 生成策略

**新记忆**：
```python
mem_id = str(uuid.uuid4())  # 随机 UUID
```

**更新记忆**：
```python
mem_id = rmeta.get("json_doc_id")  # 从 metadata 获取
```

**删除记忆**：
```python
mem_id = r.json_doc_id  # RemoveDoc 对象的字段
```

**稳定 ID**（用于去重）：
```python
def _stable_id(item: SearchItem) -> str:
    return uuid.uuid5(
        uuid.NAMESPACE_DNS,
        str((*item.namespace, item.key))
    ).hex
```

### 8.3 并发控制

**LocalReflectionExecutor**：
- 使用 `PriorityQueue` 管理任务
- 单线程 worker 顺序执行
- 同一 thread_id 的新任务取消旧任务

```python
def submit(self, payload, config, after_seconds=0, thread_id=None):
    if thread_id in self._pending_tasks:
        # 取消旧任务
        existing.cancel_event.set()
        existing.future.cancel()

    task = PendingTask(
        thread_id=thread_id,
        payload=payload,
        after_seconds=after_seconds,
        future=Future(),
        cancel_event=threading.Event()
    )
    self._pending_tasks[thread_id] = task
    self._task_queue.put((time.time() + after_seconds, task))
    return task.future
```

---

## 9. 对 remem 的启示

### 9.1 必须保留的设计

1. **双通道机制**
   - Subconscious 通道是主力（不依赖 Agent 主动调用）
   - Conscious 通道作为补充（用户显式要求时）

2. **LLM 驱动提取**
   - 不要用规则提取，用 LLM 分析对话
   - Prompt 设计是核心（`_MEMORY_INSTRUCTIONS` 值得借鉴）

3. **结构化 Schema**
   - 用 Pydantic 定义记忆类型
   - 通过 trustcall 实现并行工具调用

4. **多步 Refinement**
   - 允许 LLM 多轮迭代合并记忆
   - 第二步加入 `Done` 工具让 LLM 自主决定

5. **冲突解决**
   - LLM 决定 insert/update/delete
   - `RemoveDoc` 机制显式删除过时记忆

### 9.2 需要改进的地方

1. **搜索策略**
   - LangMem 的时间窗口搜索太简单
   - remem 应该结合语义搜索 + 重要性评分 + 时间衰减

2. **记忆重要性**
   - LangMem 没有显式的重要性评分
   - remem 应该在 schema 中加入 `importance` 字段

3. **记忆强度**
   - LangMem 没有追踪记忆的使用频率
   - remem 应该记录 `access_count` 和 `last_accessed`

4. **Prompt Optimization**
   - LangMem 的 Gradient Optimizer 很强大
   - remem 可以用来优化 system prompt

### 9.3 remem 的实现路径

**Phase 1：基础双通道**
```rust
// 1. 实现 MemoryManager（无状态）
pub struct MemoryManager {
    model: String,
    schemas: Vec<MemorySchema>,
    instructions: String,
}

impl MemoryManager {
    pub async fn extract(
        &self,
        messages: &[Message],
        existing: &[Memory]
    ) -> Result<Vec<ExtractedMemory>> {
        // 调用 LLM 提取记忆
    }
}

// 2. 实现 ReflectionExecutor（后台通道）
pub struct ReflectionExecutor {
    manager: MemoryManager,
    store: Arc<dyn MemoryStore>,
    task_queue: Arc<Mutex<PriorityQueue<Task>>>,
}

impl ReflectionExecutor {
    pub fn submit(&self, messages: Vec<Message>, after_seconds: u64) {
        // 提交后台任务
    }
}
```

**Phase 2：存储层**
```rust
// 3. 实现 MemoryStore trait
#[async_trait]
pub trait MemoryStore {
    async fn put(&self, namespace: &[String], key: &str, value: Memory);
    async fn get(&self, namespace: &[String], key: &str) -> Option<Memory>;
    async fn search(&self, namespace: &[String], query: &str, limit: usize) -> Vec<Memory>;
    async fn delete(&self, namespace: &[String], key: &str);
}

// 4. 实现 SQLite + vector search
pub struct SqliteStore {
    conn: Pool<Sqlite>,
    embedder: Box<dyn Embedder>,
}
```

**Phase 3：Prompt Optimization**
```rust
// 5. 实现 PromptOptimizer
pub struct PromptOptimizer {
    model: String,
    kind: OptimizerKind,  // Gradient / Metaprompt / PromptMemory
}

impl PromptOptimizer {
    pub async fn optimize(
        &self,
        prompt: &str,
        trajectories: &[(Vec<Message>, Option<Feedback>)]
    ) -> Result<String> {
        // 优化 prompt
    }
}
```

---

## 10. 总结

### 10.1 LangMem 的优势

1. **双通道设计**：平衡实时性和深度分析
2. **LLM 驱动**：灵活处理各种记忆类型
3. **模块化架构**：核心无状态，存储可插拔
4. **Prompt Optimization**：从反馈中学习改进行为

### 10.2 LangMem 的不足

1. **依赖 Agent 主动调用**（Conscious 通道）
2. **搜索策略简单**（时间窗口 fallback）
3. **缺少记忆重要性/强度追踪**
4. **没有记忆衰减机制**

### 10.3 remem 的方向

**核心原则**：
- **Subconscious 优先**：后台自动提取是主力
- **LLM 驱动**：不用规则，用 LLM 分析
- **质量优先**：不惜成本提升记忆质量
- **结构化**：用 schema 强制记忆类型

**关键改进**：
- 更智能的搜索（语义 + 重要性 + 时间衰减）
- 记忆强度追踪（访问频率 + 最后访问时间）
- 自动重要性评分（LLM 评估 + 用户反馈）
- Prompt 持续优化（从交互中学习）

**实现优先级**：
1. MemoryManager（核心提取逻辑）
2. ReflectionExecutor（后台通道）
3. SqliteStore（存储层）
4. 搜索优化（语义 + 重要性）
5. PromptOptimizer（行为学习）

---

## 参考资料

- [LangMem GitHub](https://github.com/langchain-ai/langmem)
- [LangMem SDK Launch Blog](https://blog.langchain.com/langmem-sdk-launch/)
- [LangMem Core Concepts](https://langchain-ai.github.io/langmem/concepts/conceptual_guide/)
- [Semantic Memory Extraction Guide](https://langchain-ai.github.io/langmem/guides/extract_semantic_memories/)
- [Episodic Memory Extraction Guide](https://langchain-ai.github.io/langmem/guides/extract_episodic_memories/)
- [Prompt Optimization Guide](https://langchain-ai.github.io/langmem/guides/optimize_memory_prompt/)
