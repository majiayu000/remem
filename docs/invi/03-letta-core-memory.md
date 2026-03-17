# Letta (MemGPT) Core Memory 深度调研

> 调研日期：2026-03-16  
> 目标：理解 Letta 的 Core Memory 机制，为 remem 提供设计参考

## 1. 如何提取

### 1.1 Agent 自主编辑 vs 自动提取

**核心设计理念**：Letta 采用 **Agent 自主编辑** 模式，而非自动提取。


**Agent 主动调用工具**：
- `core_memory_append(label, content)` - 追加内容到指定内存块
- `core_memory_replace(label, old_content, new_content)` - 精确替换内容
- `memory_insert(label, new_string, insert_line)` - 在指定行插入
- `memory_replace(label, old_string, new_string)` - 字符串替换（v2）
- `memory_rethink(label, new_memory)` - 完全重写内存块
- `memory` - Omni-tool，支持 create/str_replace/insert/delete/rename（v3）

**关键设计决策**：
1. **不依赖自动提取**：Agent 必须显式调用工具才能更新内存，系统不会自动分析对话并提取信息
2. **Agent 决定时机**：通过 system prompt 引导 Agent 在"接收到重要信息时立即更新内存"
3. **工具即接口**：内存编辑完全通过 function calling 实现，Agent 对内存有完全控制权

**来源**：
- [Letta GitHub - base.py](https://github.com/letta-ai/letta/blob/main/letta/functions/function_sets/base.py)
- [Core Memory 文档](https://docs.letta.com/guides/ade/core-memory)

### 1.2 触发条件

**主动触发（Primary Agent）**：
- System prompt 中明确指示："If you receive new important information about the user (or yourself), you immediately update your memory with core_memory_replace, core_memory_append, or archival_memory_insert"
- Agent 自行判断何时需要更新内存
- 无自动触发机制，完全依赖 Agent 的"自觉性"

**后台触发（Sleeptime Agent）**：
- 每 N 步（默认 5 步）自动调用 sleeptime agent
- Sleeptime agent 接收新消息，反思并整合到内存块
- 使用 `memory_rethink` 进行大规模重写，不阻塞用户交互

**来源**：
- [Sleeptime Agents 文档](https://docs.letta.com/guides/agents/architectures/sleeptime/)
- [Sleeptime 最佳实践](https://forum.letta.com/t/sleeptime-agents-for-memory-consolidation-best-practices-guide/154)

### 1.3 Archival Memory 的语义搜索提取

**工作流程**：
1. Agent 调用 `archival_memory_search(query, tags, top_k, start_datetime, end_datetime)`
2. 系统对 query 进行 embedding（默认使用 text-embedding-ada-002 或类似模型）
3. 向量数据库执行语义相似度搜索
4. 返回 top_k 个最相关的 passages，包含 ID、timestamp、content、tags

**检索策略**：
- 语义搜索（非关键词匹配）
- 支持 tag 过滤（any/all 模式）
- 支持时间范围过滤
- 结果按相似度排序

**来源**：
- [Archival Memory 文档](https://docs.letta.com/guides/ade/archival-memory)
- [Agent Memory 博客](https://letta.com/blog/agent-memory)

## 2. 提取什么

### 2.1 Core Memory 的内容类型

**默认内存块**：
- `human` - 存储用户信息（偏好、事实、兴趣）
- `persona` - 定义 Agent 的身份和能力

**自定义内存块**：
- 可创建任意标签的内存块（如 `knowledge`、`planning`、`todos`）
- Git-enabled agents 支持文件系统结构（`system/human.md`、`skills/debug.md`）

**内容特征**：
- 结构化文本（非 JSON，纯文本）
- 自包含的事实或摘要
- 避免对话片段，强调提炼后的知识

**来源**：
- [Core Memory 文档](https://docs.letta.com/guides/ade/core-memory)
- [Memory Blocks 博客](https://www.letta.com/blog/memory-blocks)

### 2.2 粒度

**Core Memory**：
- 默认每块 2,000 字符限制（可自定义）
- 系统常量：`CORE_MEMORY_BLOCK_CHAR_LIMIT = 100000`（最大限制）
- 超过限制会触发错误，Agent 需要压缩或拆分

**Archival Memory**：
- 以 "passages" 为单位存储
- 每个 passage 是独立的文本片段
- 支持 tags 和 timestamp 元数据
- 无明确的单个 passage 大小限制

**Recall Memory**：
- 存储完整的消息历史
- 每条消息包含 role、content、timestamp
- 自动持久化到数据库

**来源**：
- [letta/constants.py](https://github.com/letta-ai/letta/blob/main/letta/constants.py)
- [letta/schemas/memory.py](https://github.com/letta-ai/letta/blob/main/letta/schemas/memory.py)

### 2.3 Archival Memory 的内容类型

**存储内容**：
- 明确表述的知识（非原始对话历史）
- 会议笔记、项目更新、对话摘要、事件、报告
- 长期事实和上下文

**最佳实践**：
- 自包含的事实或摘要
- 添加描述性 tags 便于检索
- 避免存储对话片段

**来源**：
- [base.py - archival_memory_insert](https://github.com/letta-ai/letta/blob/main/letta/functions/function_sets/base.py#L164)

### 2.4 Recall Memory 的自动保存范围

**自动保存内容**：
- 所有对话消息（user、assistant、tool）
- 完整的交互历史
- 可搜索和检索

**持久化机制**：
- "In Letta, recall memory saves to disk automatically, while other frameworks require developers to handle persistence manually"
- 存储在数据库中，跨会话持久化

**来源**：
- [Agent Memory 博客](https://letta.com/blog/agent-memory)

## 3. 如何保存

### 3.1 Core Memory 的存储

**始终在 system prompt 中**：
- Core Memory 渲染为 `<memory_blocks>` XML 标签
- 每个 block 包含 label、description、metadata、value
- 每次 LLM 调用都包含完整的 Core Memory

**渲染格式**（标准模式）：
```xml
<memory_blocks>
The following memory blocks are currently engaged in your core memory unit:

<human>
<description>
The human block: Stores key details about the person you are conversing with...
</description>
<metadata>
- chars_current=150
- chars_limit=2000
</metadata>
<value>
User's name is Alice. Prefers dark mode. Works as a software engineer.
</value>
</human>

<persona>
<description>
The persona block: Stores details about your current persona...
</description>
<metadata>
- chars_current=200
- chars_limit=2000
</metadata>
<value>
I am a helpful AI assistant specialized in coding tasks.
</value>
</persona>

</memory_blocks>
```

**渲染格式（行号模式，Anthropic 模型）**：
```xml
<memory_blocks>
<human>
<description>...</description>
<metadata>
- chars_current=150
- chars_limit=2000
</metadata>
<warning>
# NOTE: Line numbers shown below (with arrows like '1→') are to help during editing. Do NOT include line number prefixes in your memory edit tool calls.
</warning>
<value>
1→ User's name is Alice.
2→ Prefers dark mode.
3→ Works as a software engineer.
</value>
</human>
</memory_blocks>
```

**Git-enabled 模式**：
- 内存块渲染为文件系统树 + 单独的文件标签
- 支持 YAML frontmatter
- 示例：`<system/human.md>---\ndescription: ...\n---\nContent here</system/human.md>`

**来源**：
- [letta/schemas/memory.py - Memory.compile()](https://github.com/letta-ai/letta/blob/main/letta/schemas/memory.py#L472)

### 3.2 Archival Memory 的向量化

**Embedding 模型**：
- 默认使用 OpenAI 的 text-embedding-ada-002 或类似模型
- 支持自定义 embedding 模型
- 常量：`MAX_EMBEDDING_DIM = 4096`、`DEFAULT_EMBEDDING_DIM = 1024`

**向量化流程**：
1. 文本分块（chunk）
2. 调用 embedding API 生成向量
3. 存储到向量数据库（默认 Letta 内置，支持外部数据库）

**存储结构**：
- Passages 表：id, agent_id, text, embedding, tags, created_at
- 支持语义搜索和元数据过滤

**来源**：
- [letta/constants.py](https://github.com/letta-ai/letta/blob/main/letta/constants.py#L90)
- [Archival Memory 文档](https://docs.letta.com/guides/ade/archival-memory)

### 3.3 Recall Memory 的表结构

**存储内容**：
- 完整的消息历史（Message 对象）
- 包含 role、content、timestamp、tool_calls 等

**持久化**：
- 自动保存到数据库
- 支持跨会话检索
- 通过 `conversation_search` 工具访问

**来源**：
- [Agent Memory 博客](https://letta.com/blog/agent-memory)

### 3.4 三层内存的容量限制

**Core Memory**：
- 默认每块 2,000 字符
- 最大 100,000 字符（系统限制）
- 受 LLM context window 限制

**Archival Memory**：
- 无明确容量限制
- 受数据库存储限制
- 检索时返回 top_k 结果（默认 10）

**Recall Memory**：
- 无容量限制
- 完整历史持久化
- Message buffer 有长度限制（触发 summarization）

**来源**：
- [letta/constants.py](https://github.com/letta-ai/letta/blob/main/letta/constants.py)

## 4. 如何更新

### 4.1 Core Memory 的编辑操作

**基础操作（MemGPT v1）**：
- `core_memory_append(label, content)` - 追加到末尾
- `core_memory_replace(label, old_content, new_content)` - 精确替换

**高级操作（v2）**：
- `memory_insert(label, new_string, insert_line)` - 在指定行插入
- `memory_replace(label, old_string, new_string)` - 字符串替换（带唯一性检查）
- `memory_rethink(label, new_memory)` - 完全重写

**Omni-tool（v3）**：
- `memory(command, ...)` - 统一接口
  - `command="create"` - 创建新块
  - `command="str_replace"` - 替换文本
  - `command="insert"` - 插入文本
  - `command="delete"` - 删除块
  - `command="rename"` - 重命名块

**编辑约束**：
- 必须精确匹配 old_content（不能包含行号前缀）
- 超过字符限制会报错
- 支持 Unicode 和 emoji

**来源**：
- [letta/functions/function_sets/base.py](https://github.com/letta-ai/letta/blob/main/letta/functions/function_sets/base.py)

### 4.2 Archival Memory 的追加策略

**插入操作**：
- `archival_memory_insert(content, tags)` - 添加新 passage
- 自动生成 timestamp
- 支持 tags 分类

**无更新操作**：
- Archival Memory 只支持插入，不支持修改或删除（通过 API 可以）
- Agent 无法直接删除或修改已存储的 passages

**来源**：
- [base.py - archival_memory_insert](https://github.com/letta-ai/letta/blob/main/letta/functions/function_sets/base.py#L164)

### 4.3 Recall Memory 的自动清理

**Message Buffer 管理**：
- 当消息数量超过阈值时触发 summarization
- 旧消息被压缩为摘要，保留在 summary_memory 中
- 最近的消息保留在 message buffer 中

**Summarization 触发**：
- 阈值：`context_window * 0.9`（`SUMMARIZATION_TRIGGER_MULTIPLIER`）
- 递归压缩：旧消息逐步压缩，保持最近信息的影响力

**来源**：
- [letta/constants.py](https://github.com/letta-ai/letta/blob/main/letta/constants.py#L79)
- [Agent Memory 博客](https://letta.com/blog/agent-memory)

### 4.4 三层内存的同步机制

**独立更新**：
- Core Memory、Archival Memory、Recall Memory 各自独立
- 无自动同步机制

**Agent 协调**：
- Agent 负责决定何时更新哪一层
- System prompt 引导 Agent 选择合适的存储层
- 例如：重要事实 → Core Memory，详细记录 → Archival Memory

**Sleeptime Agent 整合**：
- Sleeptime agent 可以从 Recall Memory 中提取信息
- 整合到 Core Memory 或 Archival Memory
- 实现跨层的信息流动

**来源**：
- [Sleeptime Agents 文档](https://docs.letta.com/guides/agents/architectures/sleeptime/)

## 5. 关键设计洞察

### 5.1 Self-Editing vs Auto-Extraction

**Letta 的选择**：Self-Editing（Agent 自主编辑）

**优势**：
- Agent 有完全控制权，可以精确决定存储什么
- 支持复杂的内存组织策略（重写、重组、压缩）
- 内存演化与 Agent 的"人格"绑定

**劣势**：
- 依赖 Agent 的"自觉性"，可能遗漏重要信息
- 需要强大的 system prompt 引导
- 对模型能力要求高

**竞品对比**：
- **Mem0**：自动提取 + 智能合并，降低对 Agent 的依赖
- **Zep**：混合模式，自动提取 + Agent 可编辑
- **Cognee**：自动构建知识图谱

**来源**：
- [Agent Memory Solutions 对比](https://forum.letta.com/t/agent-memory-solutions-letta-vs-mem0-vs-zep-vs-cognee/85)

### 5.2 Memory Blocks vs RAG

**Memory Blocks 的优势**：
- 始终在上下文中，无检索延迟
- Agent 可以主动编辑和组织
- 支持跨会话的持久化身份

**RAG 的局限**：
- 无状态，每次都需要检索
- Agent 无法修改检索到的内容
- 无法形成连贯的"记忆"

**Letta 的混合策略**：
- Core Memory（始终在上下文）+ Archival Memory（按需检索）
- 结合两者优势

**来源**：
- [Memory Blocks 博客](https://www.letta.com/blog/memory-blocks)

### 5.3 Sleeptime Compute 的创新

**核心思想**：
- 将计算从用户交互时转移到空闲时间
- Agent 在"睡眠"时反思和整合记忆

**实现方式**：
- Primary Agent 处理用户消息
- Sleeptime Agent 后台整合内存
- 共享内存块，异步更新

**效果**：
- 更快的响应速度（内存整合不阻塞交互）
- 更高质量的记忆（有时间深度反思）
- 成本优化（sleeptime 可用更便宜的模型）

**来源**：
- [Sleep-time Compute 博客](https://www.letta.com/blog/sleep-time-compute)
- [Sleep-time Compute 论文](https://arxiv.org/html/2504.13171)

### 5.4 Context Repositories（Git-based Memory）

**为 Coding Agents 设计**：
- 内存存储为本地文件系统
- 使用 Git 版本控制
- 支持 Unix 工具（bash、grep、scripts）

**优势**：
- 熟悉的文件系统接口
- 自动版本控制（每次修改都有 commit）
- 支持并发子 agent（通过 git worktrees）
- 离线处理和合并

**来源**：
- [Context Repositories 博客](https://www.letta.com/blog/context-repositories)

### 5.5 LoCoMo Benchmark 的发现

**关键结论**：
- 简单的文件系统存储对话历史，效果达到 74% 准确率
- 不一定需要复杂的向量搜索或知识图谱
- "Is a Filesystem All You Need?"

**启示**：
- 记忆系统的核心是**组织和访问**，而非技术复杂度
- 简单的结构化存储 + Agent 智能检索可能足够

**来源**：
- [Benchmarking AI Agent Memory 博客](https://www.letta.com/blog/benchmarking-ai-agent-memory)

## 6. 对 remem 的启示

### 6.1 必须保留的能力

1. **自动捕获**：不能完全依赖 Claude 主动调用 save_memory
   - Letta 的 self-editing 模式在 Claude Code 中不可行
   - 需要后台自动提取 + 手动保存的双通道

2. **LLM 提取质量**：这是核心竞争力
   - 不要为了省成本砍掉 LLM 提取
   - 参考 Mem0 的自动提取 + 智能合并

3. **结构化存储**：Memory Blocks 的设计值得借鉴
   - 分块存储（user、project、decisions 等）
   - 每块有字符限制和描述
   - 支持 Agent 编辑（通过 MCP 工具）

### 6.2 可以借鉴的设计

1. **三层内存架构**：
   - Core Memory（始终在上下文）
   - Archival Memory（语义搜索）
   - Recall Memory（完整历史）

2. **Sleeptime Compute**：
   - 后台整合记忆，不阻塞用户交互
   - 可以用更便宜的模型做整合

3. **Git-based Memory**（Context Repositories）：
   - 文件系统 + Git 版本控制
   - 适合 coding agents

4. **Memory Omni-tool**：
   - 统一的内存编辑接口
   - 支持 create/update/delete/rename

### 6.3 需要避免的陷阱

1. **不要依赖 Claude 主动调用**：
   - Letta 的 self-editing 模式假设 Agent 会主动更新内存
   - Claude Code 实际上不会主动调用 save_memory

2. **不要过度设计**：
   - LoCoMo benchmark 显示简单的文件系统存储效果不错
   - 不一定需要复杂的知识图谱或多层索引

3. **不要忽视成本**：
   - Sleeptime compute 的启示：可以用便宜模型做整合
   - 不是所有操作都需要最强模型

## 7. 技术细节摘要

### 7.1 函数签名

```python
# Core Memory
def core_memory_append(agent_state, label: str, content: str) -> str
def core_memory_replace(agent_state, label: str, old_content: str, new_content: str) -> str
def memory_insert(agent_state, label: str, new_string: str, insert_line: int = -1) -> str
def memory_replace(agent_state, label: str, old_string: str, new_string: str) -> str
def memory_rethink(agent_state, label: str, new_memory: str) -> str

# Archival Memory
async def archival_memory_insert(self, content: str, tags: Optional[list[str]] = None) -> Optional[str]
async def archival_memory_search(
    self, 
    query: str, 
    tags: Optional[list[str]] = None,
    tag_match_mode: Literal["any", "all"] = "any",
    top_k: Optional[int] = None,
    start_datetime: Optional[str] = None,
    end_datetime: Optional[str] = None
) -> Optional[str]

# Recall Memory
def conversation_search(
    self,
    query: Optional[str] = None,
    roles: Optional[List[Literal["assistant", "user", "tool"]]] = None,
    limit: Optional[int] = None,
    start_date: Optional[str] = None,
    end_date: Optional[str] = None
) -> Optional[str]
```

### 7.2 关键常量

```python
CORE_MEMORY_BLOCK_CHAR_LIMIT = 100000  # 最大字符限制
DEFAULT_CONTEXT_WINDOW = 128000
SUMMARIZATION_TRIGGER_MULTIPLIER = 0.9
MAX_EMBEDDING_DIM = 4096
DEFAULT_EMBEDDING_DIM = 1024
EMBEDDING_BATCH_SIZE = 200
```

### 7.3 Memory Block 结构

```python
class Block(BaseModel):
    label: str  # 块标签（如 "human", "persona"）
    value: str  # 块内容
    limit: int  # 字符限制（默认 2000）
    description: Optional[str]  # 块描述
    read_only: bool = False  # 是否只读

class Memory(BaseModel):
    blocks: List[Block]  # 内存块列表
    file_blocks: List[FileBlock]  # 文件块（用于 attached sources）
    git_enabled: bool = False  # 是否启用 Git 模式
```

## 8. 参考资料

### 核心文档
- [Letta Core Memory 文档](https://docs.letta.com/guides/ade/core-memory)
- [Letta Archival Memory 文档](https://docs.letta.com/guides/ade/archival-memory)
- [Agent Memory 博客](https://letta.com/blog/agent-memory)
- [Memory Blocks 博客](https://www.letta.com/blog/memory-blocks)

### 架构设计
- [Sleeptime Agents 文档](https://docs.letta.com/guides/agents/architectures/sleeptime/)
- [Sleep-time Compute 博客](https://www.letta.com/blog/sleep-time-compute)
- [Context Repositories 博客](https://www.letta.com/blog/context-repositories)
- [Letta V1 Agent Loop 重构](https://www.letta.com/blog/letta-v1-agent)

### 研究论文
- [MemGPT: Towards LLMs as Operating Systems](https://arxiv.org/abs/2310.08560)
- [Sleep-time Compute: Beyond Inference Scaling](https://arxiv.org/html/2504.13171)

### 竞品对比
- [Agent Memory Solutions 对比](https://forum.letta.com/t/agent-memory-solutions-letta-vs-mem0-vs-zep-vs-cognee/85)
- [Benchmarking AI Agent Memory](https://www.letta.com/blog/benchmarking-ai-agent-memory)

### 源码
- [letta/functions/function_sets/base.py](https://github.com/letta-ai/letta/blob/main/letta/functions/function_sets/base.py)
- [letta/schemas/memory.py](https://github.com/letta-ai/letta/blob/main/letta/schemas/memory.py)
- [letta/constants.py](https://github.com/letta-ai/letta/blob/main/letta/constants.py)

---

**调研完成时间**：2026-03-16  
**调研者**：Claude Opus 4.6  
**文档版本**：v1.0
