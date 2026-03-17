# 学术论文中的 LLM 记忆提取方法调研

> 调研时间：2026-03-16
> 目标：为 remem 项目设计最强的记忆提取系统

## 核心发现

学术界在 2023-2025 年间对 LLM 记忆系统进行了大量研究，核心共识是：**自动化记忆提取 + 分层存储 + 时间衰减 + 多维检索** 是构建高质量记忆系统的关键。

---

## 1. 如何提取记忆

### 1.1 Generative Agents (Stanford, 2023)

**核心方法**：观察流（Memory Stream）+ 自动反思

- **观察提取**：代理感知环境中的所有事件，自动记录为自然语言观察
- **重要性评分**：每个观察由 LLM 评分（1-10），评估其对代理行为的影响程度
- **反思触发**：当累积重要性超过阈值（如 150）时，触发反思生成
- **反思生成**：LLM 基于最近的高重要性观察，生成高层次的抽象见解

**检索公式**：
```
score = α·recency + β·importance + γ·relevance
```
- recency: 指数衰减，decay_factor = 0.995
- importance: LLM 评分（1-10）
- relevance: 向量余弦相似度

来源：[Generative Agents](https://www.emergentmind.com/topics/generative-agents), [Memory Architectures](https://arunbaby.com/ai-agents/0005-memory-architectures/)

### 1.2 Reflexion (NeurIPS 2023)

**核心方法**：Episodic Memory Buffer + 语言反思

- **反思文本生成**：任务失败后，LLM 基于任务反馈生成自然语言反思
- **Episodic Memory Buffer**：存储反思文本，按时间顺序组织
- **决策改进**：后续试验中，将相关反思注入 prompt，指导决策
- **触发时机**：任务失败或性能下降时触发

**关键洞察**：反思是"语言强化学习"——用自然语言而非数值奖励来改进行为。

来源：[Reflexion Paper](https://arxiv.org/abs/2303.11366), [Reflexion Overview](https://humaininsight.substack.com/p/advanced-overview-reflexion-a-new)

### 1.3 RET-LLM (2023)

**核心方法**：读写记忆单元 + 三元组存储

- **数据结构**：`<t₁, t₂, t₃>` 三元组（主体-关系-客体）
- **写入操作**：LLM 生成 `MEM_WRITE` API 调用，提取关系并存储
- **读取操作**：`MEM_READ` API，先精确匹配，失败则用 LSH 模糊检索
- **向量化**：存储文本 + 平均向量表示，支持语义检索

**关键洞察**：结构化存储（三元组）比纯文本更易检索和推理。

来源：[RET-LLM Paper](https://arxiv.org/html/2305.14322), [RET-LLM Overview](https://www.aimodels.fyi/papers/arxiv/ret-llm-towards-general-read-write-memory)

### 1.4 MemoryBank (2023)

**核心方法**：Ebbinghaus 遗忘曲线 + 持续更新

- **存储结构**：自然语言记忆 + 时间戳 + 重要性权重
- **遗忘机制**：基于 Ebbinghaus 遗忘曲线，记忆强度随时间衰减
- **强化机制**：重复访问的记忆强度增加，衰减速率降低
- **更新策略**：持续从新交互中提取记忆，动态更新记忆库

来源：[MemoryBank Paper](https://arxiv.org/abs/2305.10250), [MemoryBank Overview](https://paperswithcode.com/paper/memorybank-enhancing-large-language-models)

### 1.5 SynapticRAG (2024)

**核心方法**：生物突触启发 + 时间向量

- **时间表示**：记忆向量 = 语义向量 + 时间向量（spike train）
- **动态衰减**：τ(t+Δt) = τ(t) + [1-exp(-Δt)]/[1+exp(-Δt)]
- **衰减函数**：f(x) = exp(-x/τ)，τ 越大保留越久
- **检索评分**：Bscore = Tscore · (X·Y/|X|·|Y|)，结合时间和语义

**关键洞察**：时间不是元数据，而是记忆向量的一部分。

来源：[SynapticRAG Paper](https://arxiv.org/html/2410.13553v1)

---

## 2. 提取什么内容

### 2.1 观察（Observation）vs 反思（Reflection）

**观察**：
- 原始感知数据，细粒度、具体
- 例子："用户询问了关于 Rust 错误处理的问题"
- 特点：高频、低抽象、易过时

**反思**：
- 高层次抽象，从多个观察中提炼
- 例子："用户偏好函数式编程风格，避免 unwrap()"
- 特点：低频、高抽象、长期有效

**最佳实践**：两者都存储，观察用于短期上下文，反思用于长期知识。

### 2.2 记忆类型分类

#### MIRIX 六类记忆（2024）

1. **Core Memory**：代理身份 + 用户基本信息（姓名、偏好）
2. **Episodic Memory**：具体事件，包含 event_type, summary, details, actor, timestamp
3. **Semantic Memory**：抽象知识，包含 name, summary, details, source
4. **Procedural Memory**：工作流程，包含 entry_type, description, steps
5. **Resource Memory**：文档和多模态文件，包含 title, summary, resource_type, content
6. **Knowledge Vault**：敏感信息，包含 entry_type, source, sensitivity_level, secret_value

**检索策略**：元记忆管理器分析输入，路由到相关的记忆管理器，支持 embedding_match, bm25_match, string_match。

来源：[MIRIX Paper](https://arxiv.org/html/2507.07957v1), [MIRIX Architecture](https://www.emergentmind.com/topics/mirix-architecture)

#### HiMem 两层记忆（2025）

1. **Episode Memory**：细粒度交互片段
   - 双通道分割：主题转移 OR 认知显著性中断（情感/意图变化）
   - 保留原始对话上下文

2. **Note Memory**：稳定知识
   - 三阶段提取：事实单元 → 用户偏好/档案 → 规范化（去重、共指消解、时间对齐）
   - 压缩表示，加速检索

**检索策略**：
- 混合检索：同时查询两层，最大化召回率
- 最优努力检索：先查 Note，不足时才查 Episode

来源：[HiMem Paper](https://arxiv.org/html/2601.06377)

#### H-MEM 四层记忆（2024）

1. **Domain Layer**：最高抽象（章节级别）
2. **Category Layer**：中间抽象（小节级别）
3. **Memory Trace Layer**：细化层级（小小节级别）
4. **Episode Layer**：底层具体内容（完整交互记录）

**检索策略**：自上而下索引路由，通过位置索引指针逐层导航，复杂度从 O(a·10⁶·D) 降至 O((a+k·300)·D)。

来源：[H-MEM Paper](https://arxiv.org/html/2507.22925v1)

### 2.3 重要性评分

**Generative Agents 方法**：
- LLM 评分（1-10）："On a scale of 1 to 10, where 1 is purely mundane and 10 is extremely poignant, rate the likely poignancy of the following piece of memory."
- 评分标准：对代理行为的影响程度

**动态记忆巩固方法**（2024）：
- 相关性：向量余弦相似度（0-1）
- 经过时间：秒为单位
- 回忆频率：影响衰减常数 g_n
- 公式：p_n(t) = [1 - exp(-r·e^(-t/g_n))] / (1 - e^(-1))

来源：[Dynamic Memory Paper](https://arxiv.org/html/2404.00573v1)

---

## 3. 如何保存记忆

### 3.1 数据结构

**文本 + 向量 + 元数据**：
```rust
struct Memory {
    id: Uuid,
    content: String,           // 自然语言内容
    embedding: Vec<f32>,       // 向量表示
    timestamp: DateTime,       // 创建时间
    last_accessed: DateTime,   // 最后访问时间
    access_count: u32,         // 访问次数
    importance: f32,           // 重要性评分（1-10）
    memory_type: MemoryType,   // 观察/反思/事实/偏好
    source: String,            // 来源（对话ID/文件路径）
}
```

**三元组结构**（RET-LLM）：
```rust
struct Triplet {
    subject: String,
    relation: String,
    object: String,
    embedding: Vec<f32>,  // 平均向量
}
```

### 3.2 向量化表示

**Embedding 模型选择**：
- 学术界常用：OpenAI text-embedding-ada-002, Sentence-BERT
- 开源替代：BGE, E5, Instructor

**时间向量融合**（SynapticRAG）：
- 记忆向量 = 语义向量 + 时间向量（spike train）
- 时间向量编码：二维数组（刺激值 + 时间戳）

### 3.3 时间戳和权重

**时间戳类型**：
- 创建时间：记忆首次生成的时间
- 最后访问时间：最近一次检索的时间
- 最后更新时间：内容修改的时间

**权重计算**：
- 初始权重：重要性评分
- 动态调整：基于访问频率和用户反馈
- 衰减函数：指数衰减或 sigmoid 衰减

### 3.4 层级组织

**短期 → 长期转换**：
1. **工作记忆**（Working Memory）：当前对话上下文，存储在 prompt 中
2. **短期记忆**（Short-term Memory）：最近的观察，存储在数据库中
3. **长期记忆**（Long-term Memory）：反思和知识，经过压缩和抽象

**转换触发**：
- 时间阈值：24 小时后从短期转长期
- 重要性阈值：高重要性观察直接进入长期
- 访问频率：频繁访问的短期记忆提升为长期

来源：[Memory Mechanisms](https://www.emergentmind.com/topics/memory-mechanisms-in-llm-based-agents-c6936a2e-2296-46de-b469-040d6767712a)

---

## 4. 如何更新记忆

### 4.1 记忆合并和压缩

**HiMem 的 Memory Reconsolidation**：
- **触发条件**：Note Memory 检索不足 AND Episode Memory 提供充分证据
- **冲突分类**：
  - 独立：ADD 操作（添加新知识）
  - 可扩展：UPDATE 操作（补充现有知识）
  - 矛盾：DELETE 操作（删除过时知识）
- **不可变性**：Episode Memory 仅追加，保留时间完整性

**递归记忆巩固**（2025）：
- 将相关记忆单元整合为更高层次的抽象表示
- 减少冗余，提升检索效率
- 异步执行，不阻塞主流程

来源：[Efficient Lifelong Memory](https://arxiv.org/html/2601.02553v1)

### 4.2 反思触发条件

**累积重要性阈值**（Generative Agents）：
- 累积最近观察的重要性评分
- 超过阈值（如 150）时触发反思
- 反思后重置累积值

**认知显著性中断**（HiMem）：
- 检测情感变化（sentiment shift）
- 检测意图变化（intent shift）
- 检测主题转移（topic shift）

**任务失败触发**（Reflexion）：
- 任务执行失败
- 性能指标下降
- 用户明确反馈错误

### 4.3 遗忘机制

**时间衰减函数**：
- 指数衰减：f(t) = exp(-t/τ)
- Ebbinghaus 曲线：快速初始衰减 + 缓慢长期衰减
- 动态 τ：频繁访问增加 τ，减缓遗忘

**智能修剪**（Intelligent Decay）：
- 低重要性 + 长时间未访问 → 删除
- 高重要性 + 长时间未访问 → 压缩为摘要
- 频繁访问 → 保留完整内容

来源：[Memory Management](https://arxiv.org/html/2509.25250v1)

### 4.4 检索时的动态重排序

**多维评分融合**：
```
final_score = w1·semantic_score + w2·recency_score + w3·importance_score + w4·frequency_score
```

**自适应权重学习**（2024）：
- 使用 MoE 门控函数自动学习权重
- 对比学习优化检索函数
- SFT/DPO 优化记忆利用

来源：[Adaptive Memory Framework](https://arxiv.org/html/2508.16629)

**Learned Retrieval Weights**（Ditto）：
- 轻量级 MLP 动态调整权重
- 训练数据：用户反馈 + 任务性能
- 推理速度：亚毫秒级

来源：[Ditto Blog](https://heyditto.ai/blog/learned-retrieval-weights-how-ditto-picks-the-right-memories)

---

## 5. 关键设计决策

### 5.1 自动 vs 手动提取

**学术界共识**：自动提取是主力，手动保存是补充。

**自动提取的优势**：
- 不依赖用户"自觉性"
- 捕获隐式知识（用户未意识到的模式）
- 持续运行，不遗漏信息

**手动保存的场景**：
- 用户明确标记重要信息
- 敏感信息需要用户确认
- 纠正自动提取的错误

来源：[LMKit Memory Modes](https://docs.lm-kit.com/lm-kit-net/api/LMKit.Agents.Memory.MemoryExtractionMode.html)

### 5.2 实时 vs 批量提取

**实时提取**（每次交互后）：
- 优点：信息新鲜，上下文完整
- 缺点：延迟增加，成本高

**批量提取**（定期或触发）：
- 优点：成本低，可离线处理
- 缺点：信息滞后，可能遗漏

**混合策略**：
- 高重要性事件 → 实时提取
- 常规交互 → 批量提取（每 N 条消息或每 M 分钟）

### 5.3 结构化 vs 非结构化存储

**结构化**（三元组、字段）：
- 优点：易检索、易推理、易更新
- 缺点：提取成本高，可能丢失细节

**非结构化**（自然语言）：
- 优点：保留完整上下文，提取简单
- 缺点：检索依赖向量相似度，精度有限

**最佳实践**：混合存储
- 原始内容：非结构化（完整保留）
- 索引字段：结构化（加速检索）
- 例子：MIRIX 的 summary + details 设计

### 5.4 向量检索 vs 全文检索

**向量检索**（Embedding + ANN）：
- 优点：语义理解，支持模糊匹配
- 缺点：精确匹配差，冷启动问题

**全文检索**（BM25, Elasticsearch）：
- 优点：精确匹配强，速度快
- 缺点：无语义理解，同义词问题

**混合检索**（MIRIX 方法）：
- 支持 embedding_match, bm25_match, string_match
- 根据查询类型自动选择最佳方法
- 精确查询 → BM25，模糊查询 → Embedding

---

## 6. 对 remem 的启示

### 6.1 必须实现的功能

1. **自动记忆提取**（不能依赖 Claude 主动调用 save_memory）
   - 每次对话结束后自动提取
   - 使用 LLM 评估重要性
   - 区分观察和反思

2. **分层存储**（参考 HiMem 或 MIRIX）
   - 短期：原始对话片段
   - 长期：压缩的知识和偏好
   - 核心：用户身份和项目上下文

3. **多维检索**（不只是向量相似度）
   - 语义相关性（embedding）
   - 时间近期性（exponential decay）
   - 重要性评分（LLM 评分）
   - 访问频率（强化机制）

4. **记忆更新**（不是只追加）
   - 冲突检测（新旧记忆矛盾）
   - 合并压缩（相似记忆整合）
   - 遗忘机制（低价值记忆删除）

### 6.2 技术选型建议

**LLM 调用**：
- 重要性评分：轻量级模型（Claude Haiku / GPT-4o-mini）
- 反思生成：强模型（Claude Opus / GPT-4）
- 成本控制：批量处理 + 缓存

**向量数据库**：
- 本地优先：SQLite + sqlite-vec
- 云端备选：Qdrant, Weaviate
- 混合检索：向量 + BM25

**存储结构**：
- 原始对话：JSON 或 MessagePack
- 记忆索引：SQLite 关系表
- 向量索引：专用向量库

### 6.3 实现优先级

**P0（必须有）**：
1. 自动观察提取（每次对话后）
2. 重要性评分（LLM 评分 1-10）
3. 向量检索（embedding + 余弦相似度）
4. 时间衰减（exponential decay）

**P1（应该有）**：
1. 反思生成（累积重要性触发）
2. 记忆分层（短期/长期/核心）
3. 混合检索（向量 + BM25）
4. 记忆合并（去重 + 压缩）

**P2（可以有）**：
1. 多维评分融合（自适应权重）
2. 冲突检测和解决
3. 用户反馈强化
4. 跨会话记忆共享

### 6.4 避免的陷阱

1. **不要砍掉 LLM 提取**：成本不是问题，质量才是目标
2. **不要依赖手动保存**：Claude 不会主动调用 save_memory
3. **不要只存储向量**：保留原始文本，向量只是索引
4. **不要忽略时间维度**：recency 和 importance 同样重要
5. **不要过早优化**：先实现基础功能，再优化性能

---

## 7. 参考文献

### 核心论文

1. **Generative Agents** (Stanford, 2023)
   - [Paper](https://arxiv.org/abs/2304.03442)
   - [Overview](https://www.emergentmind.com/topics/generative-agents)

2. **Reflexion** (NeurIPS 2023)
   - [Paper](https://arxiv.org/abs/2303.11366)
   - [GitHub](https://github.com/noahshinn/reflexion)

3. **RET-LLM** (2023)
   - [Paper](https://arxiv.org/html/2305.14322)
   - [Overview](https://www.aimodels.fyi/papers/arxiv/ret-llm-towards-general-read-write-memory)

4. **MemoryBank** (2023)
   - [Paper](https://arxiv.org/abs/2305.10250)
   - [Papers with Code](https://paperswithcode.com/paper/memorybank-enhancing-large-language-models)

5. **SynapticRAG** (2024)
   - [Paper](https://arxiv.org/html/2410.13553v1)

6. **MIRIX** (2024)
   - [Paper](https://arxiv.org/html/2507.07957v1)
   - [Architecture](https://www.emergentmind.com/topics/mirix-architecture)

7. **HiMem** (2025)
   - [Paper](https://arxiv.org/html/2601.06377)

8. **H-MEM** (2024)
   - [Paper](https://arxiv.org/html/2507.22925v1)

### 综述和博客

- [Memory Architectures](https://arunbaby.com/ai-agents/0005-memory-architectures/)
- [Memory Mechanisms in LLM Agents](https://www.emergentmind.com/topics/memory-mechanisms-in-llm-based-agents-c6936a2e-2296-46de-b469-040d6767712a)
- [AI Agent Memory Survey](https://www.graphlit.com/blog/survey-of-ai-agent-memory-frameworks)
- [Learned Retrieval Weights (Ditto)](https://heyditto.ai/blog/learned-retrieval-weights-how-ditto-picks-the-right-memories)
- [Temporal Vector Stores](https://scrapingant.com/blog/temporal-vector-stores-indexing-scraped-data-by-time-and)

### 最新进展

- [Adaptive Memory Framework](https://arxiv.org/html/2508.16629) (2024)
- [Efficient Lifelong Memory](https://arxiv.org/html/2601.02553v1) (2025)
- [Dynamic Memory Recall](https://arxiv.org/html/2404.00573v1) (2024)
- [Memory Consolidation](https://arxiv.org/html/2412.07393v1) (2024)

---

## 8. 生产环境实践

### 8.1 成本优化策略

**批量处理**（降低 API 调用成本）：
- 累积 N 条消息后批量提取记忆
- 使用更便宜的模型（Haiku/GPT-4o-mini）做重要性评分
- 使用强模型（Opus/GPT-4）做反思生成
- 典型成本：$0.01-0.05 per conversation

**缓存策略**：
- 向量 embedding 缓存（相同文本不重复编码）
- LLM 响应缓存（相同 prompt 复用结果）
- 检索结果缓存（相同查询复用）

**延迟优化**：
- 异步提取：对话结束后后台处理
- 增量更新：只处理新增内容
- 分层检索：先查快速索引，必要时才查向量

来源：[Production LLM Optimization](https://tech.growthx.ai/posts/how-to-optimize-llm-inference-in-production), [Memory Production Guide](https://genmind.ch/posts/Your-LLM-Has-Amnesia-A-Production-Guide-to-Memory-That-Actually-Works/)

### 8.2 质量评估指标

**LongMemEval Benchmark**（5 个核心任务）：
1. **信息提取**：从对话中提取关键事实
2. **多会话推理**：跨会话关联信息
3. **知识更新**：检测和更新过时信息
4. **时间推理**：理解事件的时间顺序
5. **遗忘检测**：识别应该遗忘的信息

**MemBench 评估维度**：
- **召回率**（Recall）：相关记忆被检索的比例
- **精确率**（Precision）：检索结果的相关性
- **新鲜度**（Freshness）：记忆的时效性
- **一致性**（Consistency）：记忆之间的逻辑一致性

**实际指标**：
- 用户满意度：用户对记忆准确性的评分
- 任务成功率：依赖记忆的任务完成率
- 冲突率：新旧记忆矛盾的频率
- 遗忘率：重要信息被错误删除的频率

来源：[LongMemEval](https://www.emergentmind.com/topics/longmemeval-benchmark), [MemBench](https://aclanthology.org/2025.findings-acl.989/), [Memory Evaluation](https://arxiv.org/html/2507.05257v1)

### 8.3 触发条件调优

**阈值类型**：

1. **累积重要性阈值**（Generative Agents）：
   - 默认值：150
   - 调优方法：根据对话长度和领域调整
   - 短对话（<10 轮）：降低到 100
   - 长对话（>50 轮）：提高到 200

2. **时间阈值**：
   - 最小间隔：避免频繁触发（如 5 分钟）
   - 最大间隔：确保定期提取（如 24 小时）

3. **回忆概率阈值**（动态记忆）：
   - 默认值：0.86
   - 公式：p(t) = [1 - exp(-r·e^(-t/g_n))] / (1 - e^(-1))
   - 调优：根据记忆重要性调整

**自适应调整**（Adaptive Memory Admission Control）：
- 监控记忆库大小和检索性能
- 动态调整准入阈值
- 高价值记忆降低阈值，低价值记忆提高阈值

来源：[Dynamic Memory Recall](https://arxiv.org/html/2404.00573v1), [Adaptive Admission Control](https://arxiv.org/html/2603.04549v1)

### 8.4 Prompt Engineering 最佳实践

**重要性评分 Prompt**：
```
On a scale of 1 to 10, where 1 is purely mundane (e.g., "hello")
and 10 is extremely poignant (e.g., user shares a major life decision),
rate the likely importance of the following observation for future interactions:

Observation: {observation_text}

Consider:
- Does this reveal user preferences or constraints?
- Will this information be useful in future conversations?
- Does this represent a significant event or decision?

Respond with only a number from 1 to 10.
```

**反思生成 Prompt**：
```
Given the following recent observations (sorted by importance):

{top_k_observations}

Generate 3-5 high-level insights that synthesize these observations.
Each insight should:
- Be abstract and generalizable (not just restating observations)
- Capture patterns or preferences
- Be useful for guiding future interactions

Format: One insight per line, starting with "Insight:"
```

**记忆提取 Prompt**（ProMem 方法）：
```
Extract key information from this conversation that should be remembered:

Conversation:
{conversation_text}

Extract:
1. Facts: Concrete information (names, dates, preferences)
2. Preferences: User likes/dislikes, constraints
3. Context: Project goals, ongoing tasks
4. Decisions: Important choices made

For each item, provide:
- Type: [fact/preference/context/decision]
- Content: Brief description
- Importance: [high/medium/low]
```

来源：[Prompt Engineering 2025](https://www.getmaxim.ai/articles/a-practitioners-guide-to-prompt-engineering-in-2025/), [ProMem](https://arxiv.org/html/2601.04463v1), [Context Engineering](https://orchestrator.dev/blog/2025-02-14-context-engineering-blog-article)

### 8.5 MCP Server 实现参考

**现有实现**：

1. **@modelcontextprotocol/server-memory**（官方）：
   - 使用本地知识图谱
   - 支持实体和关系存储
   - 简单的检索接口

2. **claude-memory-mcp**（社区）：
   - 分层记忆（短期/长期/核心）
   - 语义搜索
   - 自动记忆管理

3. **Memory Agent MCP**：
   - 向量数据库（Voyage AI embeddings）
   - 存储对话、代码片段、项目上下文
   - 跨会话检索

4. **Claude Code Memory Server**：
   - Neo4j 图数据库
   - 追踪活动、决策、学习模式
   - 上下文记忆

**架构模式**：
```
MCP Server
├── Tools (Claude 调用)
│   ├── save_memory(text, type, importance)
│   ├── search_memory(query, limit)
│   ├── update_memory(id, new_content)
│   └── delete_memory(id)
├── Resources (Claude 读取)
│   ├── recent_memories (最近 N 条)
│   ├── important_memories (高重要性)
│   └── project_context (当前项目)
└── Prompts (模板)
    ├── extract_memory (提取 prompt)
    └── generate_reflection (反思 prompt)
```

来源：[MCP Memory Servers](https://mcpservers.org/), [Claude Code Memory Guide](https://crunchtools.com/how-to-give-claude-code-persistent-memory/)

---

## 9. 实现路线图

### Phase 1: 基础自动提取（2 周）

**目标**：替换当前的 save_memory 手动保存，实现自动捕获。

**任务**：
1. 实现对话结束钩子（检测会话结束）
2. 实现重要性评分（LLM 评分 1-10）
3. 实现观察提取（自然语言描述）
4. 存储到 SQLite（text + embedding + metadata）
5. 实现基础检索（向量相似度 + 时间衰减）

**验证**：
- 每次对话后自动生成 3-5 条观察
- 重要性评分合理（重要事件 >7，日常对话 <5）
- 检索能找到相关历史记忆

### Phase 2: 分层记忆（3 周）

**目标**：实现短期/长期/核心三层记忆。

**任务**：
1. 实现记忆类型分类（observation/reflection/fact/preference）
2. 实现反思触发（累积重要性 >150）
3. 实现反思生成（LLM 生成高层次见解）
4. 实现记忆转换（短期 → 长期）
5. 实现核心记忆（用户身份 + 项目上下文）

**验证**：
- 反思质量高（抽象、可泛化）
- 长期记忆稳定（不频繁变化）
- 核心记忆准确（用户偏好、项目目标）

### Phase 3: 多维检索（2 周）

**目标**：实现 recency + importance + relevance 融合检索。

**任务**：
1. 实现时间衰减函数（exponential decay）
2. 实现多维评分融合（加权求和）
3. 实现混合检索（向量 + BM25）
4. 实现检索结果重排序
5. 调优权重参数（α, β, γ）

**验证**：
- 最近的重要记忆排名靠前
- 旧的低重要性记忆排名靠后
- 检索精度提升（对比纯向量检索）

### Phase 4: 记忆更新（3 周）

**目标**：实现记忆合并、冲突检测、遗忘机制。

**任务**：
1. 实现冲突检测（新旧记忆矛盾）
2. 实现记忆合并（相似记忆整合）
3. 实现遗忘机制（低价值记忆删除）
4. 实现用户反馈强化（点赞/点踩）
5. 实现记忆统计和可视化

**验证**：
- 冲突能被检测并解决
- 重复记忆被合并
- 低价值记忆被自动删除
- 用户反馈影响记忆权重

### Phase 5: 优化和评估（2 周）

**目标**：成本优化、性能优化、质量评估。

**任务**：
1. 实现批量处理（降低 API 调用）
2. 实现缓存策略（embedding + LLM 响应）
3. 实现异步提取（不阻塞对话）
4. 实现质量评估（召回率、精确率）
5. 性能基准测试

**验证**：
- 成本降低 50%+
- 延迟 <100ms（检索）
- 召回率 >80%，精确率 >70%

---

## 附录：关键公式汇总

### 检索评分公式

**Generative Agents**：
```
score = α·recency + β·importance + γ·relevance
recency = exp(-decay_factor · hours_since_access)
importance = LLM_score(1-10)
relevance = cosine_similarity(query_embedding, memory_embedding)
```

**SynapticRAG**：
```
Bscore = Tscore · (X·Y / |X|·|Y|)
Tscore = DTW(spike_train_query, spike_train_memory)
```

### 时间衰减公式

**指数衰减**：
```
f(t) = exp(-t/τ)
```

**动态 τ 更新**：
```
τ(t+Δt) = τ(t) + [1-exp(-Δt)] / [1+exp(-Δt)]
```

**记忆巩固概率**：
```
p_n(t) = [1 - exp(-r·e^(-t/g_n))] / (1 - e^(-1))
g_n = g_(n-1) + S(t)
S(t) = (1-e^(-t)) / (1+e^(-t))
```

### 重要性评分

**累积重要性**：
```
cumulative_importance = Σ importance_score_i
trigger_reflection = cumulative_importance > threshold (e.g., 150)
```

**动态权重调整**：
```
weight_new = weight_old · feedback_multiplier
feedback_multiplier = {
  1.5  if user_approved,
  1.0  if no_feedback,
  0.5  if user_rejected
}
```
