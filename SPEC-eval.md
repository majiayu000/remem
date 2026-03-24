# remem 评估方法调研报告

> 调研日期：2026-03-24
> 目标：为 remem（Claude Code 记忆系统）建立量化评估体系

---

## 一、学术 Benchmark 全景

### 1.1 LoCoMo — Very Long-Term Conversational Memory (ACL 2024)

**来源**：Snap Research, Maharana et al.
**论文**：[arxiv.org/abs/2402.17753](https://arxiv.org/abs/2402.17753)
**官网**：[snap-research.github.io/locomo](https://snap-research.github.io/locomo)

**数据集规模**：
- 10 段超长对话，每段 ~600 轮、~16K tokens、跨 32 sessions
- LLM 生成 + 人工验证，基于 persona 和时间事件图

**评估任务**：
| 任务类别 | 说明 |
|---------|------|
| Single-hop QA | 单 session 内事实检索 |
| Multi-hop QA | 跨 session 信息综合 |
| Temporal QA | 时间推理（日期/顺序/间隔） |
| Open-domain QA | 需要世界知识的问题 |
| Adversarial QA | 不可回答的问题（测试幻觉抵抗） |
| Event Summarization | 因果和时间关系提取 |
| Multimodal Dialogue | 多模态对话生成 |

**核心指标**：
- F1 Score（答案预测）
- BLEU-1（文本相似度）
- **LLM-as-Judge**（事实准确性、相关性、完整性、上下文适当性）— 已成为主流评估方式
- MM-Relevance Score（多模态生成）

**关键发现**：
- RAG + 长上下文 LLM 比基线提升 22-66%，但仍落后人类 56%
- 长上下文模型在对抗性问题上严重幻觉
- 基于断言的数据库检索效果最佳

**remem 适用性**：⭐⭐⭐⭐ — 跨 session 记忆召回是 remem 核心场景，但 LoCoMo 是对话型，需要适配到代码开发场景。

---

### 1.2 LongMemEval — Long-Term Interactive Memory (ICLR 2025)

**来源**：Wu et al.
**论文**：[arxiv.org/abs/2410.10813](https://arxiv.org/abs/2410.10813)
**GitHub**：[github.com/xiaowu0162/LongMemEval](https://github.com/xiaowu0162/LongMemEval)

**数据集规模**：
- 500 条人工标注问题
- LongMemEvalS：~115K tokens/历史，30-40 sessions
- LongMemEvalM：~1.5M tokens/历史，~500 sessions

**五大核心记忆能力**：
| 能力 | 说明 | remem 对应场景 |
|------|------|---------------|
| Information Extraction | 从历史中回忆具体细节 | 回忆之前的技术决策/架构选择 |
| Multi-Session Reasoning | 跨 session 综合推理 | 跨项目/跨会话知识关联 |
| Temporal Reasoning | 理解时间维度（显式时间 + 时间戳） | "上周我们修了什么 bug" |
| Knowledge Updates | 识别信息变更并动态更新 | preference 变更、架构演进 |
| Abstention | 拒绝回答不存在的信息 | 不编造之前没做过的事 |

**三阶段框架**：
1. **Indexing**：round-level 粒度优于 session-level
2. **Retrieval**：fact-augmented key expansion 提升 recall 4%、accuracy 5%；time-aware query expansion 提升 temporal reasoning recall 7-11%
3. **Reading**：Chain-of-Note + 结构化 JSON 格式提升 accuracy 10pp

**关键发现**：
- 商业聊天助手在持续交互中记忆准确率下降 30-60%
- GPT-4o 仅达到 30-70% 准确率

**remem 适用性**：⭐⭐⭐⭐⭐ — 五大能力维度完美对应 remem 需要的评估维度。可直接参考其框架设计 remem 的评估。

---

### 1.3 MemoryBench — Memory and Continual Learning (arXiv 2025)

**来源**：Ai et al.
**论文**：[arxiv.org/abs/2510.17281](https://arxiv.org/abs/2510.17281)

**特色**：测试从用户反馈中持续学习的能力（procedural memory）。

**数据集**：11 个公开数据集，3 个领域（开放/法律/学术），4 种任务格式，~20K 测试用例。

**评估维度**：
- Declarative memory（语义+情景）
- Procedural memory（通过用户反馈学习的技能）
- 用户反馈模拟框架（满意度评分 → 概率行为模型）

**关键发现**：
- **Mem0、A-Mem、MemoryOS 均无法稳定超越简单的 RAG baseline**
- 现有系统把所有输入当 declarative memory，缺乏 procedural memory 处理
- Mem0 延迟不稳定，MemoryOS 每条记忆构建 >17 秒

**remem 适用性**：⭐⭐⭐ — procedural memory 对 remem 重要（用户偏好学习），但数据集偏通用 QA。

---

### 1.4 MemoryAgentBench — Incremental Multi-Turn Interactions (ICLR 2026)

**来源**：Hu et al., HUST
**论文**：[arxiv.org/abs/2507.05257](https://arxiv.org/abs/2507.05257)

**四大核心能力**：
| 能力 | 说明 | 测试方法 |
|------|------|---------|
| Accurate Retrieval (AR) | 精确检索事实 | RULER-QA, NIAH-MQ, LongMemEval 等 5 个数据集 |
| Test-Time Learning (TTL) | 部署时学习新技能 | 分类任务 + 推荐评估 |
| Long-Range Understanding (LRU) | 长程抽象理解 | 摘要任务 |
| Conflict Resolution (CR) | 矛盾信息处理 | FactConsolidation（单跳+多跳） |

**增量注入评估**：文档按 512/4096 token chunk 顺序注入（非一次性全量），模拟真实增量学习。

**核心指标**：SubEM（精确匹配）、ROUGE F1、Recall/Accuracy、GPT-4o 评判、Recall@5

**关键发现**：
- 所有方法在 **Conflict Resolution 上表现极差**（多跳最高仅 6% 准确率）
- Embedding-based RAG 在纯检索上优于图结构增强
- 商业记忆系统（Mem0、MemGPT）全线表现有限

**remem 适用性**：⭐⭐⭐⭐ — Conflict Resolution 直接对应 remem 的记忆去重和更新场景。增量注入模式也贴合 remem 的工作方式。

---

### 1.5 MEMTRACK — Multi-Platform Dynamic Agent Environments (NeurIPS 2025)

**来源**：Patronus AI
**论文**：[arxiv.org/abs/2510.01353](https://arxiv.org/abs/2510.01353)

**特色**：模拟真实组织工作流，跨 Slack/Linear/Git 等平台。

**评估指标**：
- **Correctness**：记忆正确性
- **Efficiency**：记忆操作效率
- **Redundancy**：冗余记忆检测

**关键发现**：GPT-5 在 MEMTRACK 上也只达到 60% Correctness。

**remem 适用性**：⭐⭐⭐⭐⭐ — 这个 benchmark 最接近 remem 的实际场景（跨多 session 的软件开发工作流），指标也非常贴合。

---

### 1.6 ConvoMem — Conversation Memory Scaling (arXiv 2025)

**来源**：Salesforce
**论文**：[arxiv.org/abs/2511.10523](https://arxiv.org/abs/2511.10523)
**数据集**：[HuggingFace](https://huggingface.co/datasets/Salesforce/ConvoMem)

**规模**：75,336 QA 对，6 个类别。

**关键发现**：
- 前 30 次对话：全上下文方式即可达到 70-82% 准确率
- 30-150 次对话：长上下文方式仍然可行
- 150+ 次对话：需要 RAG/混合方案
- Mem0 在 <150 次对话上仅达到 30-45%

**remem 适用性**：⭐⭐⭐ — 提供了关于记忆系统 ROI 的重要参考（何时需要记忆系统 vs 纯上下文）。

---

## 二、竞品评估方法

### 2.1 Mem0

**评估论文**：[Mem0: Building Production-Ready AI Agents with Scalable Long-Term Memory](https://arxiv.org/abs/2504.19413)（2025.04）

**评估方法**：
- 在 LoCoMo 上评估，使用 LLM-as-Judge（10 次独立运行取均值 ± 标准差）
- 对比 6 类 baseline：LoCoMo 原始、ReadAgent、MemoryBank、MemGPT、A-Mem、RAG 变体、全上下文

**关键指标**：
| 指标 | 数值 |
|------|------|
| Overall LLM-as-Judge | 67.13% |
| Single-hop J | 67.13 ± 0.65 |
| Multi-hop J | 51.15 ± 0.31 |
| Open-domain J | 72.93 ± 0.11 |
| Temporal J | 55.51 ± 0.34 |
| p95 Latency | 0.200s（vs 全上下文 17.12s） |
| Token 消耗 | ~1,764/对话（vs 全上下文 26,031） |

**Mem0g（图增强版）**：时间推理 58.13% vs OpenAI 的 21.71%

**Memory 提取流程**：
1. 给定消息对 (m_t-1, m_t)，检索对话摘要 + 最近 10 条消息
2. LLM 提取显著事实 Ω
3. 每个事实与 top-10 语义相似的现有记忆比较
4. 决定操作：ADD / UPDATE / DELETE / NOOP

---

### 2.2 Zep / Graphiti

**评估论文**：[Zep: A Temporal Knowledge Graph Architecture for Agent Memory](https://arxiv.org/abs/2501.13956)（2025.01）

**评估方法**：
- DMR (Deep Memory Retrieval) benchmark（500 条对话，60 消息/对话）
- LongMemEval benchmark

**DMR 结果**：Zep 94.8% vs MemGPT 93.4%（但 DMR 已被认为过于简单）

**LongMemEval 结果**：准确率提升最高 18.5%，延迟降低 90%

**评估指标**：Accuracy、F1、Precision、Recall + LLM Judge

**局限性**：DMR 仅测试单轮事实检索，无法评估复杂记忆理解。

---

### 2.3 Letta (MemGPT)

**评估体系**：

**A. Letta Leaderboard**（模型记忆能力排行榜）
- 评估维度：Core Memory（读/写/更新）+ Archival Memory（外部记忆管理）
- 使用虚构 QA 数据集，GPT-4.1 作为评判
- 额外惩罚低效的记忆操作
- Top 模型：Claude Sonnet 4（带 extended thinking）、GPT-4.1

**B. LoCoMo Filesystem 实验**
- 将对话历史存为文件，仅用 grep/search_files/open/close 工具
- 结果：74.0% 准确率（超过 Mem0 的 68.5%）
- **关键洞察**：简单文件系统 > 专用记忆工具库

**C. Context-Bench**（2025 下半年）
- 评估 agentic context engineering 能力
- Filesystem Suite：文件操作链、实体追踪、多步信息检索
- Skill Use Suite：从技能库中发现和加载相关技能
- 使用 SQL 数据库生成有验证答案的测试问题

---

### 2.4 LangMem

**评估方法**：主要依赖 LoCoMo benchmark，无独立评估论文。

**记忆类型**：semantic（事实）、procedural（技能）、episodic（经历）

**优化算法**：metaprompt（反思式）、gradient（批评+提案分离）、simple prompt_memory

---

### 2.5 Hindsight

**论文**：[Hindsight is 20/20](https://arxiv.org/abs/2512.12818)（2025.12）

**评估结果**：
- LongMemEval：91.4%（Gemini-3 backbone）— SOTA
- LoCoMo：89.61% — SOTA
- 开源 20B 模型：83.6%（超过 GPT-4o 全上下文基线）

**核心操作评估**：retain（记忆写入）→ recall（记忆检索）→ reflect（记忆反思/偏好融合）

---

### 2.6 Cognee

**评估方法**：
- HotPotQA 子集（24 道多跳题），45 次重复运行
- 四项指标：Exact Match、F1、DeepEval Correctness、Human-like Correctness
- GPT-4o 作为评判

**结果**：Cognee (0.93 human-like correctness) > LightRAG > Graphiti > Mem0

---

## 三、适用于 remem 的评估维度

基于调研，为 remem 设计以下六维评估体系：

### 3.1 记忆召回率 (Memory Recall)

**定义**：在正确时机想起正确记忆的能力。

**对应 benchmark 维度**：
- LongMemEval 的 Information Extraction + Multi-Session Reasoning
- MemoryAgentBench 的 Accurate Retrieval

**测试设计**：
```
场景：在 session N 做了技术决策 D，在 session N+K 问"之前关于 X 的决策是什么"
指标：Recall@K = 正确召回数 / 应召回总数
目标：Recall@5 >= 0.9, Recall@20 >= 0.95
```

### 3.2 记忆精度 (Memory Precision)

**定义**：召回的记忆是否真的与当前查询相关。

**对应 benchmark 维度**：
- MEMTRACK 的 Redundancy 指标
- RAG 评估的 Contextual Precision

**测试设计**：
```
场景：查询 "auth 模块" 时返回的记忆中有多少确实与 auth 相关
指标：Precision@K = 相关结果数 / 返回结果总数
目标：Precision@5 >= 0.8
```

### 3.3 记忆时效性 (Temporal Awareness)

**定义**：新记忆优先于旧记忆，理解时间维度。

**对应 benchmark 维度**：
- LongMemEval 的 Temporal Reasoning
- LoCoMo 的 Temporal QA
- MemoryAgentBench 的 Conflict Resolution
- Mem0g 的时间推理评估

**测试设计**：
```
场景 A：session 1 选择了方案 A，session 5 改为方案 B，查询时应返回方案 B
场景 B："上周做了什么" vs "上个月做了什么" 能区分时间范围
指标：Temporal Accuracy = 时间正确的回答数 / 总问题数
目标：>= 0.85
```

### 3.4 跨会话连贯性 (Cross-Session Coherence)

**定义**：多次会话后记忆保持一致和完整。

**对应 benchmark 维度**：
- LongMemEval 的 Knowledge Updates
- MemoryAgentBench 的 Long-Range Understanding
- ConvoMem 的 Changing Facts 类别

**测试设计**：
```
场景：跨 10 个 session 积累关于项目 X 的记忆，最终查询应包含所有关键事实
指标：Coherence Score = (一致记忆数 - 矛盾记忆数) / 总记忆数
目标：>= 0.9
```

### 3.5 记忆去重 (Deduplication Quality)

**定义**：相似记忆被正确合并，不重复冗余。

**对应 benchmark 维度**：
- MEMTRACK 的 Redundancy
- Mem0 的 ADD/UPDATE/DELETE/NOOP 操作评估

**测试设计**：
```
场景：连续 3 个 session 讨论同一个 bug 的不同方面，应合并为 1 条记忆而非 3 条
指标：Dedup Ratio = 1 - (重复记忆数 / 总记忆数)
目标：>= 0.95
```

### 3.6 上下文注入质量 (Context Injection Quality)

**定义**：注入的记忆是否真正帮助了当前任务。

**对应 benchmark 维度**：
- Letta 的 Context-Bench（agentic context engineering）
- Cognee 的 Human-like Correctness
- LongMemEval 的 Abstention（不应注入不相关的记忆）

**测试设计**：
```
场景：开发者开始新任务时，remem 自动注入的上下文是否包含必要的历史决策
指标：
  - Helpfulness = 有帮助的注入 / 总注入数
  - Noise Ratio = 无关注入 / 总注入数
  - Abstention Rate = 正确不注入 / 应不注入的场景数
目标：Helpfulness >= 0.7, Noise Ratio <= 0.15
```

---

## 四、remem Benchmark 方案

### 4.1 数据集构建

由于 remem 面向代码开发场景，现有 benchmark 无法直接使用。需要构建 **CodeMemBench**：

#### 数据来源

1. **合成会话**：用 LLM 生成多 session 的代码开发对话
   - persona：开发者 + Claude Code
   - 事件图：项目创建 → 功能开发 → bug 修复 → 架构重构 → 偏好变更
   - 每个项目 20-50 sessions，跨 1-3 个月

2. **真实会话脱敏**：从实际 remem 使用数据中提取（脱敏后）

#### 问题类别（对应六维评估）

| 类别 | 示例问题 | 对应维度 |
|------|---------|---------|
| Fact Recall | "我们选择了什么数据库？" | 3.1 召回率 |
| Decision Context | "为什么决定用 SQLite 而不是 Postgres？" | 3.1 + 3.6 |
| Cross-Project | "另一个项目中类似的 auth 实现是怎么做的？" | 3.4 连贯性 |
| Temporal | "上周修了哪些 bug？" | 3.3 时效性 |
| Knowledge Update | "最初用 REST，后来改成 gRPC，当前方案是？" | 3.3 + 3.4 |
| Adversarial | "我们用过 GraphQL 吗？"（实际没用过） | 3.6 Abstention |
| Preference | "我喜欢用什么代码风格？" | 3.1 + 3.4 |
| Bug Pattern | "这个 error 之前出现过吗？怎么修的？" | 3.1 + 3.3 |

### 4.2 评估流程

```
┌─────────────────────────────────────────────┐
│  Phase 1: Memory Ingestion                  │
│  ─ 按时间序注入 N 个 session 的对话数据      │
│  ─ 模拟 remem observe → summarize 流程      │
│  ─ 记录：记忆条数、去重率、写入延迟          │
└──────────────────────┬──────────────────────┘
                       ▼
┌─────────────────────────────────────────────┐
│  Phase 2: Memory Query                      │
│  ─ 对每个问题调用 remem search/timeline     │
│  ─ 记录：返回的记忆条目、检索延迟            │
└──────────────────────┬──────────────────────┘
                       ▼
┌─────────────────────────────────────────────┐
│  Phase 3: Answer Generation                 │
│  ─ 将检索到的记忆作为上下文，LLM 生成回答    │
│  ─ 与 ground truth 对比                     │
└──────────────────────┬──────────────────────┘
                       ▼
┌─────────────────────────────────────────────┐
│  Phase 4: Scoring                           │
│  ─ LLM-as-Judge 评估 (GPT-4o / Claude)     │
│  ─ 传统指标：F1, EM, ROUGE                  │
│  ─ 记忆系统专用指标：Recall@K, Precision@K  │
│  ─ 效率指标：token 消耗、延迟               │
└─────────────────────────────────────────────┘
```

### 4.3 评分标准

#### A. LLM-as-Judge 评分模板

```
你是记忆系统质量评判官。给定一个问题、ground truth 答案和系统回答，
评估系统回答的质量。

评分维度（每项 1-5 分）：
1. 事实准确性：回答中的事实是否正确？
2. 完整性：是否覆盖了 ground truth 中的所有关键信息？
3. 相关性：回答是否紧扣问题？
4. 时效性：是否使用了最新的信息（而非过时的）？
5. 无幻觉：是否包含 ground truth 中不存在的编造信息？

问题：{question}
Ground Truth：{ground_truth}
系统回答：{system_answer}
检索到的记忆：{retrieved_memories}

请给出每个维度的评分和理由，以及总分（25 分满分）。
```

#### B. 效率指标

| 指标 | 计算方法 | 目标 |
|------|---------|------|
| Memory Token Ratio | 检索 tokens / 原始对话 tokens | < 0.1 |
| Search Latency p50 | 中位检索延迟 | < 100ms |
| Search Latency p95 | 95 分位延迟 | < 500ms |
| Ingestion Throughput | sessions/minute | > 10 |

#### C. 综合评分公式

```
Score = 0.30 × Recall@5
      + 0.20 × Precision@5
      + 0.15 × Temporal_Accuracy
      + 0.15 × Coherence_Score
      + 0.10 × Dedup_Ratio
      + 0.10 × (1 - Noise_Ratio)
```

### 4.4 Baseline 对比

| 对比项 | 说明 |
|--------|------|
| No Memory | 纯 LLM 无任何记忆（下界） |
| Full Context | 全部对话历史塞进上下文（理论上界，不经济） |
| RAG Baseline | BM25 / 向量检索最近 K 条消息 |
| remem v_current | 当前 remem 版本 |

### 4.5 具体测试用例（20 条核心用例）

```yaml
# === Recall ===
- id: R01
  type: single_hop_recall
  setup: "Session 3 中讨论并选择了 SQLite 作为数据库"
  query: "我们用的什么数据库？"
  answer: "SQLite"

- id: R02
  type: multi_hop_recall
  setup: "Session 2 选择了 Rust，Session 5 加了 async runtime tokio"
  query: "我们的 async 技术栈是什么？"
  answer: "Rust + tokio"

- id: R03
  type: cross_project_recall
  setup: "项目 A 中实现了 JWT auth，项目 B 中提问"
  query: "之前其他项目怎么做 auth 的？"
  answer: "项目 A 使用 JWT auth"

# === Temporal ===
- id: T01
  type: knowledge_update
  setup: "Session 1 用 REST API，Session 8 改为 gRPC"
  query: "我们的 API 通信协议是什么？"
  answer: "gRPC（从 REST 迁移过来的）"

- id: T02
  type: temporal_range
  setup: "Session 10-12（上周）修了 3 个 bug"
  query: "上周修了哪些 bug？"
  answer: "列出 3 个 bug 的描述"

- id: T03
  type: recency_priority
  setup: "Session 3 设置 max_retries=3，Session 15 改为 max_retries=5"
  query: "max_retries 设置的多少？"
  answer: "5（最新值）"

# === Precision ===
- id: P01
  type: relevance_filter
  setup: "有 auth、database、UI 三个模块的记忆各 10 条"
  query: "auth 模块的设计决策"
  answer: "只返回 auth 相关记忆，不混入其他模块"

# === Coherence ===
- id: C01
  type: cross_session_consistency
  setup: "Session 1-10 逐步构建了完整的项目架构"
  query: "项目整体架构是什么？"
  answer: "综合所有 session 的架构信息，无矛盾"

# === Deduplication ===
- id: D01
  type: merge_similar
  setup: "Session 3、5、7 都讨论了缓存策略的不同方面"
  query: "缓存策略是什么？"
  answer: "合并后的完整缓存策略，非 3 条重复记忆"

# === Abstention ===
- id: A01
  type: unanswerable
  setup: "从未讨论过 GraphQL"
  query: "我们的 GraphQL schema 是怎么设计的？"
  answer: "历史记录中未发现关于 GraphQL 的讨论"

# === Preference ===
- id: PR01
  type: preference_recall
  setup: "用户多次表达偏好 snake_case 命名"
  query: "用户的代码命名偏好是什么？"
  answer: "snake_case"

- id: PR02
  type: preference_global
  setup: "在项目 A 表达了'不要用 ORM'的偏好"
  query: "在项目 B 中，用户对 ORM 的态度？"
  answer: "用户偏好不使用 ORM（全局偏好）"

# === Bug Pattern ===
- id: B01
  type: bug_recall
  setup: "Session 6 修了一个死锁 bug，根因是嵌套 RwLock"
  query: "之前遇到过死锁问题吗？"
  answer: "是的，Session 6 修了嵌套 RwLock 导致的死锁"

- id: B02
  type: bug_prevention
  setup: "Session 6 修了死锁 bug 并记录了教训"
  query: "当前代码中有嵌套锁获取，有什么风险？"
  answer: "之前因为嵌套 RwLock 出过死锁，需要注意"

# === Decision Context ===
- id: DC01
  type: decision_rationale
  setup: "Session 4 讨论了 SQLite vs Postgres，选择了 SQLite 因为单文件部署"
  query: "为什么选 SQLite？"
  answer: "因为单文件部署简单，不需要额外数据库进程"

- id: DC02
  type: rejected_alternatives
  setup: "Session 4 拒绝了 Postgres 和 MySQL"
  query: "当时还考虑了什么替代方案？"
  answer: "考虑过 Postgres 和 MySQL，但因为部署复杂度被否决"

# === Architecture ===
- id: AR01
  type: architecture_evolution
  setup: "Session 1: monolith → Session 10: 拆出 worker → Session 15: 加 MCP server"
  query: "系统架构是怎么演进的？"
  answer: "从 monolith 开始，先拆出 worker，后加 MCP server"

# === Efficiency ===
- id: E01
  type: token_efficiency
  setup: "20 个 session，共 100K tokens 的对话"
  query: "任意问题"
  measure: "检索注入的 token 数 vs 总对话 token 数"
  target: "注入 tokens < 总对话 tokens 的 10%"
```

### 4.6 自动化测试集成

建议在 `tests/` 目录创建评估框架：

```
tests/
  eval/
    fixtures/        # 合成对话数据（JSON）
    questions.yaml   # 测试问题和 ground truth
    eval_runner.rs   # 评估运行器
    judge.rs         # LLM-as-Judge 评分
    report.rs        # 评估报告生成
```

运行方式：
```bash
cargo test --test eval -- --ignored   # 评估测试（需要 LLM API key）
```

---

## 五、关键参考文献索引

| 名称 | 年份 | 会议/平台 | 核心贡献 |
|------|------|----------|---------|
| LoCoMo | 2024 | ACL | 首个大规模长对话记忆 benchmark |
| LongMemEval | 2025 | ICLR | 五维记忆能力评估框架 |
| MemoryBench | 2025 | arXiv | procedural memory + 用户反馈模拟 |
| MemoryAgentBench | 2025→2026 | ICLR | 增量注入 + Conflict Resolution |
| MEMTRACK | 2025 | NeurIPS | 跨平台组织工作流记忆评估 |
| ConvoMem | 2025 | arXiv | 75K QA 对 + 记忆系统 ROI 分析 |
| Mem0 Paper | 2025 | arXiv | LLM-as-Judge 评估方法 + 效率指标 |
| Zep Paper | 2025 | arXiv | 时间知识图谱 + DMR/LongMemEval 评估 |
| Hindsight | 2025 | arXiv | retain/recall/reflect 三操作 + SOTA |
| Letta Leaderboard | 2025 | Blog | 模型记忆能力排行榜 + Context-Bench |
| Memory Survey | 2025 | arXiv | 46 人综述，forms/functions/dynamics 分类 |

---

## 六、下一步行动建议

1. **短期（1-2 周）**：手写 20 条核心测试用例（上面 4.5 节），在当前 remem 上跑一遍基线
2. **中期（1 个月）**：构建 CodeMemBench 合成数据集，100+ QA 对，自动化评估流程
3. **长期（持续）**：
   - 接入 LongMemEval 的公开数据集做通用记忆能力测试
   - 基于 MEMTRACK 的设计思路构建开发者场景评估
   - 建立 CI 中的回归评估（每次改动跑评估，防止记忆质量退化）
