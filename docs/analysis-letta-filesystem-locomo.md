# Letta Filesystem LoCoMo 方法深度分析

> 调研日期: 2026-03-25
> 目标: 理解 Letta 用简单文件系统操作在 LoCoMo 上达到 74% 的原因，为 remem 提供改进方向

---

## 一、Letta 的具体实现

### 1.1 存储格式

**每个 session 一个 .txt 文件**，不是每轮一个文件。

文件存放在 `data/sample_{id}/` 目录下，文件名格式:
- `session` 策略: 以 session 时间戳命名，如 `2022-05-04.txt`
- `secom` 策略: 以 segment 编号命名，如 `2022-05-04_001.txt`

文件内容格式:
```
Timestamp: 2022/05/04
Alice: Hey, I just got back from my trip to India!
Bob: That's awesome! How was it?
Alice: It was amazing, I visited the Taj Mahal...
```

关键点: **原始对话文本直接存储，不做任何 LLM 提炼/摘要**。

### 1.2 Chunking 策略

代码支持 4 种策略:
| 策略 | 说明 | 粒度 |
|------|------|------|
| `turn` | 每轮对话一个文件 | 最细 |
| `session` | 每个 session 一个文件（**默认，跑出 74% 的策略**） | 中等 |
| `time_window` | 每 10 轮一个文件 | 固定窗口 |
| `secom` | GPT-4o-mini 做话题分割，每个话题一个文件 | 语义连贯 |

### 1.3 Agent 可用的工具

最终跑分使用的工具配置:

```python
tools=["search_files", "answer_question"]
# "grep", "open_file", "close_file" 被注释掉了
```

**实际只用了 2 个工具**:
1. `search_files` — Letta 内置的语义搜索工具（基于 text-embedding-3-large），对上传文件的内容做向量搜索
2. `answer_question` — 简单的回答终止工具，返回 LLM 决定的最终答案

注意: 博客里说有 `grep`，但代码中 grep 被注释掉了。实际跑分版本**只有语义搜索**。

### 1.4 Tool Rules（Agent 行为约束）

```python
tool_rules=[
    InitToolRule(tool_name="search_files"),       # 必须先搜索
    ContinueToolRule(tool_name="grep"),            # grep 后可继续
    ContinueToolRule(tool_name="search_files"),    # 搜索后可继续搜索
    TerminalToolRule(tool_name="answer_question"), # 回答后终止
]
```

这意味着:
1. Agent **必须先调用 search_files**（不能直接回答）
2. Agent **可以多次搜索**，直到找到满意的信息
3. Agent **自己决定搜索什么 query**
4. Agent **自己决定搜索几次**
5. 最后调用 answer_question 终止

### 1.5 System Prompt

```
You are tasked with answering questions about conversation history.
The conversation history will be provided as a series of files.
Read through the conversation context carefully before answering any questions.
You can use search_files tools to gather information from the conversation context.
Please provide precise answers to the questions, do not hallucinate, and note the time as well!
The timestamp of a conversation is in the file name.
Always refer to a file to answer the question, do not make up information.
Avoid general searches, like just searching for a person's name.
Try directly search the question in the conversation first, before you try some other queries.
Your final answer_question tool call should be a precise and concise answer.
When answering a question about time, use a specific time.
If the memories contain contradictory information, prioritize the most recent memory.
If there is a question about time references, calculate the actual date based on the memory timestamp.
Always convert relative time references to specific dates, months, or years.
Focus only on the content of the memories from both speakers.
Do not confuse character names mentioned in memories with the actual users.
```

### 1.6 Embedding 配置

```python
EmbeddingConfig(
    embedding_model="text-embedding-3-large",
    embedding_endpoint_type="openai",
    embedding_dim=1536,
    embedding_chunk_size=100000,  # 非常大的 chunk size
)
```

### 1.7 LLM 模型

- Agent（搜索+回答）: **GPT-4o-mini**
- SeCom 话题分割: GPT-4o-mini
- 内容摘要（可选，默认关闭）: GPT-4o-mini

---

## 二、为什么 Letta 比 remem 高 17 个点

### 2.1 分数对比

| 类别 | remem (56.8%) | Mem0 (论文) | Letta Filesystem (74%) |
|------|:---:|:---:|:---:|
| single-hop | 67.1% | 67.1% | 未公开 |
| multi-hop | 39.0% | 51.2% | 未公开 |
| temporal | 53.9% | 55.5% | 未公开 |
| open-domain | 28.1% | 72.9% | 未公开 |
| **overall** | **56.8%** | **67.1%** | **74.0%** |

Letta 没有公开 per-category 分数，只公布了 overall 74%。

### 2.2 核心差异分析

#### 差异 1: Agent-Driven Retrieval vs Pre-defined Retrieval（关键差异）

**remem 的检索流程**:
```
问题 → FTS5 搜索（OR + synonym expansion + RRF） → top-K 结果 → LLM 生成答案
```
检索策略是**预定义的**，只执行一次，query 就是原始问题。

**Letta 的检索流程**:
```
问题 → LLM 自主决定搜索 query → 语义搜索 → 看结果 → 可能再搜索不同 query → ... → LLM 生成答案
```
检索策略是**LLM 自主决定的**，可以迭代多次，query 由 LLM 改写。

Letta 博客原文的关键洞察:
> "Agents can generate their own queries rather than simply searching the original questions
> (e.g., transforming 'How does Calvin stay motivated when faced with setbacks?' into
> 'Calvin motivation setbacks'), and they can continue searching until the right data is found."

这是**最关键的差异**。remem 的搜索是一次性的，Letta 让 LLM 像人类一样迭代搜索。

#### 差异 2: 向量搜索 vs 全文搜索

**remem**: FTS5 全文搜索（关键词匹配）
**Letta**: text-embedding-3-large 向量搜索（语义匹配）

FTS5 对于需要语义理解的问题（如 "How does Calvin stay motivated" 搜不到 "Calvin said he runs every morning to keep his spirits up"）天然劣势。

#### 差异 3: 原始对话 vs LLM 提炼后的记忆

**remem 的 ingest 流程**:
```
对话轮次 → "[2022/05/04] Alice: ..." → 直接存为 memory → FTS5 索引
```
存储的是格式化后的原始文本，title 是人工构造的 `"speaker - session N (dia_id)"`。

**Letta 的 ingest 流程**:
```
对话轮次 → 按 session 整合为完整对话文件 → embedding 索引
```
存储的也是原始文本，但**每个文件包含完整 session 上下文**，而不是逐轮拆散。

这意味着 Letta 的搜索结果保留了**对话连贯性**——搜到一句话就能看到前后文。remem 每条 memory 是孤立的。

#### 差异 4: 搜索时有 LLM 参与 vs 无 LLM 参与

**remem**: 搜索完全在 Rust/SQLite 层完成，无 LLM 参与
**Letta**: 搜索由 GPT-4o-mini Agent 驱动，LLM 决定搜什么、搜几次、何时停止

---

## 三、关键结论

### 3.1 事实（来源: 代码分析）

1. [来源: locomo_benchmark.py:857-860] Letta 实际只用 `search_files` + `answer_question` 两个工具，grep 被注释掉
2. [来源: locomo_benchmark.py:799-805] 使用 text-embedding-3-large 做语义搜索
3. [来源: locomo_benchmark.py:863-868] Tool rules 强制 Agent 先搜索，可以迭代搜索，最后回答终止
4. [来源: locomo_benchmark.py:879] 默认使用 session-level chunking（每个 session 一个文件）
5. [来源: locomo_agent.txt] System prompt 只有 12 行，非常简洁
6. [来源: remem eval results] remem 在 multi-hop(39%) 和 open-domain(28.1%) 上严重落后

### 3.2 推断

1. [基于: 差异分析] Agent-driven iterative retrieval 是 Letta 74% 的最大贡献因素（置信度: 高）
   - 理由: remem 的 single-hop(67%) 和 Mem0 的 single-hop(67%) 几乎相同，说明单次检索能力差距不大。差距主要在 multi-hop 和 open-domain，这些正是迭代搜索能大幅改善的类别

2. [基于: embedding vs FTS5] 向量搜索对 open-domain 类别贡献最大（置信度: 中）
   - 理由: open-domain 问题需要语义理解，FTS5 关键词匹配天然弱

3. [基于: session-level chunking] 保留完整 session 上下文有助于 multi-hop（置信度: 中）
   - 理由: multi-hop 需要关联同一 session 内的多条信息，拆散存储会丢失关联性

### 3.3 建议

1. **[前提: agent-driven retrieval 是关键差异]** remem 的 MCP search 工具应支持 LLM 迭代调用
   - 当前 remem 的 eval pipeline 是: query → search → answer，应改为: query → LLM(search → 判断 → 可能再搜) → answer
   - 替代方案: 在 search 工具内部实现 query rewriting（不需要多轮调用，但需要 LLM）

2. **[前提: 向量搜索有价值]** 为 remem 添加 embedding-based 搜索作为 FTS5 的补充
   - 风险: 增加外部依赖（需要 embedding API 或本地模型）
   - 替代方案: 用 LLM 做 query expansion（把语义问题转换为多个关键词查询），保持纯 FTS5

3. **[前提: session-level context 有价值]** ingest 时保留 session-level 粒度的原始对话
   - 当前 remem 按单轮存储，搜到一条记忆没有前后文
   - 可以在搜索结果中附带同 session 的邻近记忆

---

## 四、对 remem LoCoMo 评估流程的改进方向

### 4.1 快速改进（不改 remem 核心代码）

修改 `eval/locomo/run_locomo.py` 的评估流程:

```
当前: question → remem search(question) → top-K → LLM answer
改进: question → LLM rewrite query → remem search(rewritten) →
      LLM 判断是否需要更多搜索 → 可能再搜 → LLM answer
```

这样可以在不修改 remem 搜索引擎的情况下，测试 agent-driven retrieval 对分数的影响。

### 4.2 中期改进（改 remem 搜索）

1. 搜索结果附带 context window（同 session 的前后 N 条记忆）
2. 添加 embedding 搜索通道（与 FTS5 做 RRF 融合）
3. 支持 LLM query rewriting（在 MCP search 工具内部）

### 4.3 长期方向

Agent-driven retrieval 的启示是:**记忆系统的 "检索" 不应该是一个静态函数调用，而应该是一个 LLM 驱动的多步过程**。

这与 remem 当前的架构有根本冲突——remem 是一个被动的 MCP server，由 Claude Code 调用 search 工具获取结果。要实现 agent-driven retrieval，需要:
- 要么让 Claude Code 自己多次调用 search（依赖 Claude 的主动性，可能不可靠）
- 要么在 remem 内部实现一个 mini-agent loop（search 工具内部用 LLM 做迭代搜索）

---

## 五、参考资料

- [Letta 博客: Benchmarking AI Agent Memory](https://www.letta.com/blog/benchmarking-ai-agent-memory)
- [Letta LoCoMo 代码](https://github.com/letta-ai/letta-leaderboard/blob/main/leaderboard/locomo/locomo_benchmark.py)
- [Letta Agent Prompt](https://github.com/letta-ai/letta-leaderboard/blob/main/leaderboard/locomo/locomo_agent.txt)
- [Mem0 论文 LoCoMo 分数](https://arxiv.org/abs/2504.19413)
- [LoCoMo 原始论文](https://arxiv.org/abs/2402.17753)
