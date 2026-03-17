# AI 记忆系统竞品对比调研（2026-03-15）

## 调研范围

10+ 个项目，覆盖 3 类：通用记忆框架、Claude Code 记忆工具、开源知识管理框架。

---

## 一、核心架构对比

| 维度 | **Mem0** | **Zep** | **Letta (MemGPT)** | **Engram** | **Cognee** | **LangMem** | **remem** |
|------|---------|---------|-------------------|-----------|-----------|------------|----------|
| **架构模式** | Python SDK + API | 知识图谱服务 | Agent 框架 | Go 二进制 + MCP | Python SDK | LangChain 集成 | Rust 二进制 + MCP |
| **存储** | 向量 DB + 图 DB + SQLite 审计 | Neo4j 图数据库 | SQLite/PostgreSQL + 向量 | SQLite + FTS5 | 向量 DB + 图 DB | 向量存储 (BaseStore) | SQLite + FTS5 trigram |
| **搜索** | 向量语义 + Rerank + 图增强 | 语义 + BM25 + 图遍历 (BFS) | 语义 (Archival) + 直接访问 (Core) | FTS5 + 三层递进 | 向量 + 图遍历 + 时间过滤 | 纯向量语义 | FTS5 trigram + LIKE 回退 |
| **触发方式** | 主动 API 调用 | 主动 API 调用 | Agent tool call | Agent 主动调用 MCP | API 调用 | 对话流被动 + 主动工具 | **全自动 hooks（被动）** |
| **LLM 依赖** | 提取+决策双阶段 | 13+ 并发 prompt | Core memory 编辑 | 摘要生成 | ECL 三阶段 | 提取+优化 | 观察+摘要 |
| **外部依赖** | 向量 DB + 图 DB | Neo4j | 无（SQLite 模式） | 无 | 向量 DB + 图 DB | LangGraph | **无** |
| **部署复杂度** | 中等 | 高 | 中等 | 低 | 中等 | 中等 | **最低（单二进制）** |

---

## 二、记忆生命周期管理

| 维度 | **Mem0** | **Zep** | **Letta** | **Engram** | **remem** |
|------|---------|---------|----------|-----------|----------|
| **去重** | LLM 决策 (ADD/UPDATE/DELETE/NOOP) | 三层实体解析（精确→模糊→LLM） | Memory Defragmentation | Hash + 主题键更新 | Delta 去重 + 文件覆盖检测 |
| **冲突解决** | LLM 判断矛盾→DELETE old | Bitemporal invalid_at 标记 | last-write-wins | 主题键版本更新 | stale 标记 |
| **过期/压缩** | 访问衰减（60 天） | 无自动 GC，invalid_at 逻辑过期 | 无显式机制 | 软删除 | **4 阶段管道：pending→observe→compress→cleanup（90 天 TTL）** |
| **时间感知** | 创建/更新/访问时间 | **Bitemporal（Event + Ingestion）** | 无 | 创建时间 | 创建时间 + 时间衰减排序 |

---

## 三、记忆层级模型

### Letta 三层模型（最经典）

```
Core Memory（始终在 context window）
  ↕ Agent 主动编辑（insert/replace/rethink）
Archival Memory（外部向量 DB，语义搜索）
  ↕ Agent 按需查询
Recall Memory（对话历史表，自动保存）
```

### Zep 三层子图模型

```
Episode Subgraph（原始消息，非损失存储）
  → LLM 处理 →
Semantic Entity Subgraph（实体+关系+事实，带 valid_at/invalid_at）
  → 社区检测 →
Community Subgraph（强连接实体簇摘要）
```

### Mem0 混合模型

```
向量 DB（语义搜索，主存储）
  + 图 DB（实体关系，可选增强）
  + SQLite 审计日志（操作历史）
```

### remem 管道模型

```
pending（入队 <1ms）
  → observations（Stop hook 批量 AI 处理 ≤15 条）
  → session_summaries（会话级摘要）
  → compressed（>100 条时自动合并最旧 30 条）
  → cleanup（90 天 TTL）
  + workstreams（跨会话任务追踪）
```

---

## 四、检索策略对比

| 策略 | 使用者 | 优点 | 缺点 |
|------|--------|------|------|
| **向量语义搜索** | Mem0, Zep, Letta, Cognee, LangMem | 理解语义，不依赖关键词 | 需要 embedding 模型，成本高 |
| **BM25 全文搜索** | Zep (融合) | 精确关键词匹配 | 无语义理解 |
| **FTS5 trigram** | remem, Engram | 子串匹配，CJK 友好，零外部依赖 | 无语义理解 |
| **图遍历 (BFS)** | Zep | 发现关联关系，多跳推理 | 需要图 DB |
| **三层递进披露** | Engram | Token 效率最高（节省 99%） | 需多次查询 |
| **Reranking (RRF/MMR)** | Mem0, Zep | 提升精度 | 额外延迟 |
| **LIKE 回退** | remem | 覆盖短 token（<3 字符） | O(n) 扫描 |

---

## 五、Claude Code 记忆 MCP 生态

调研 7 个 Claude Code 记忆项目：memory-mcp, claude-mem, basic-memory, claude-memory-mcp, mcp-memory-keeper, mcp-memory, remem。

**核心发现**：
- **所有竞品都需要手动调用工具**，remem 是唯一全自动被动系统
- **无竞品有生命周期管理**（压缩/过期/清理）
- **无竞品有速率限制和成本追踪**
- **无竞品有工作流追踪**（WorkStream）

---

## 六、remem 可借鉴的设计模式

### 优先级 1：三层递进披露（来自 Engram）
搜索返回紧凑 ID+标题 → 时间线 → 完整内容。减少 context window 占用。

### 优先级 2：主题键去重（来自 Engram）
同主题观察用版本更新而非追加，防止话题重复堆积。

### 优先级 3：Bitemporal 时间模型（来自 Zep）
区分 Event Time（事实发生时间）和 Ingestion Time（记录时间），支持点对时间查询。

### 优先级 4：Core Memory 块（来自 Letta）
小型始终在 context 内的状态板（~2000 字符），Agent 主动编辑。remem 的 WorkStream 已部分实现此模式。

### 优先级 5：Prompt 梯度优化（来自 LangMem）
从记忆自动改进 system prompt，双 LLM 回路迭代。

---

## 七、remem 当前竞争优势

1. **全自动被动捕获** — 零用户操作，hooks 驱动
2. **单二进制零依赖** — 无需向量 DB/图 DB/外部服务
3. **4 阶段生命周期** — 唯一有压缩+过期管道的系统
4. **速率限制+成本追踪** — 3 层 gate + Worker 双检 + token 预算
5. **WorkStream 追踪** — 整个生态的空白填补
6. **CJK 搜索支持** — trigram tokenizer + LIKE 回退

## 八、记忆保存机制深度对比

### 8.1 什么时候保存（触发时机）

| 项目 | 触发方式 | 具体时机 |
|------|---------|---------|
| **Mem0** | 每轮对话后 `memory.add()` | 应用层在每次 user↔assistant 交互后主动调用 |
| **Zep** | 每条消息 `thread.add_messages()` | 应用层在每条消息发送后主动调用 |
| **Letta** | Agent 自己决定 | Agent 在对话中判断值得记住时调用 `core_memory_insert/replace` |
| **Engram** | Agent 自己决定 | Agent 判断后调用 `mem_save(title, summary)` |
| **LangMem** | 实时 + 后台双通道 | 实时：Agent 调用 `mem_save_tool`；后台：Memory Manager 自动分析对话 |
| **claude-mem** | 4 个 hooks 自动 | PostToolUse 异步入队 → Stop hook 触发 AI 批量处理 |
| **remem** | 4 个 hooks 自动 | PostToolUse 入队 → Stop hook 触发 flush+summarize |

**三种触发模式**：

```
模式 A: 应用层强制（每轮都存）    → Mem0, Zep
  优点：不漏
  缺点：冗余多，需后续 LLM 过滤去噪
  成本：每轮 1 次 LLM 调用

模式 B: Agent 自主判断（按需存）   → Letta, Engram
  优点：最精准，只存有价值的信息
  缺点：依赖 Agent 判断能力，可能漏存关键信息
  成本：Agent 决定时才调用

模式 C: 全自动被动（hooks 拦截）   → claude-mem, remem
  优点：零用户操作，不依赖 Agent 判断
  缺点：需要过滤噪音（read-only 命令、重复操作等）
  成本：批量延迟处理，每 session 1 次 LLM 调用
```

### 8.2 怎么保存（处理管道）

| 项目 | 原始输入 | 处理步骤 | 最终存储形态 |
|------|---------|---------|------------|
| **Mem0** | 消息对 | ① LLM 提取候选事实 → ② 向量搜索已有记忆 → ③ LLM 决策 ADD/UPDATE/DELETE/NOOP | 单条事实（"用户喜欢 Python"） |
| **Zep** | 消息 | ① 实体提取 → ② 关系提取 → ③ 实体解析去重 → ④ 边去重 → ⑤ 事实合成 → ⑥ 社区检测 | 知识图谱节点+边+事实 |
| **Letta** | Agent 显式调用 | 直接写入，无中间处理 | Core Memory 块（≤2000 字符文本） |
| **Engram** | Agent 显式调用 | Hash 去重 + 主题键匹配（同主题更新而非追加） | 观察记录（title+summary+metadata） |
| **LangMem** | 对话历史 | ① LLM 提取 episodic/semantic 记忆 → ② 合并冲突 → ③ 向量化 | 向量化记忆片段 |
| **claude-mem** | 工具操作 | ① 入队 → ② AI 批量提炼（≤15 条）→ ③ 向量化存入 Chroma | 观察（摘要+事实+决策）+ 向量 |
| **remem** | 工具操作 | ① 入队 → ② AI 批量提炼（≤15 条）→ ③ FTS5 索引 | 观察（title+narrative+facts+concepts） |

**处理复杂度光谱**：

```
零处理（直接写入）：           Letta, Engram
  → 最快，<1ms，但 Agent 要自己组织内容质量

单阶段 LLM：                  LangMem
  → 提取+分类一步完成，1 次 LLM 调用

双阶段 LLM：                  Mem0
  → 提取 → 去重决策，精度最高但成本 2x

多阶段流水线：                 Zep（6 步，13+ 并发 prompt）
  → 最完整（实体+关系+事实+社区），但最慢最贵

批量延迟处理：                 claude-mem, remem
  → 攒一批再处理，不阻塞会话，成本最优
  → ≤15 条/批，1 次 LLM 调用处理整批
```

### 8.3 什么时候用（检索与注入）

| 项目 | 检索时机 | 检索方式 | 注入方法 |
|------|---------|---------|---------|
| **Mem0** | 生成回复前 `memory.search()` | 向量语义 + Rerank + 图增强 | 拼入 system prompt |
| **Zep** | 生成回复前调用搜索 API | 语义 + BM25 + 图遍历 → RRF 融合 | 构造 context block 注入 prompt |
| **Letta** | 不需要检索 | Core Memory 始终在 context | 每步编译到 system prompt 前缀 |
| **Engram** | Agent 按需调用 `mem_search` | FTS5 全文搜索 | MCP tool 返回结果 |
| **LangMem** | Agent 按需调用 | 向量语义搜索 | tool 返回 + 自动更新 system prompt |
| **claude-mem** | SessionStart hook 自动 | FTS5 + Chroma 向量混合 | hook 输出注入对话开头 |
| **remem** | SessionStart hook 自动 | FTS5 trigram / LIKE 回退 | hook 输出注入对话开头 |

**三种注入模式**：

```
模式 X: 始终在 context（无需检索）    → Letta Core Memory
  优点：零延迟，永不遗漏
  缺点：容量有限（~2000 字符/块），挤占 context window
  适用：高频访问的关键状态

模式 Y: 按需检索（Agent/应用主动查）  → Mem0, Zep, Engram, LangMem
  优点：灵活，精确，不浪费 context
  缺点：依赖调用方记得去查，可能遗漏
  适用：大量历史记忆中精确查找

模式 Z: 会话启动自动注入             → claude-mem, remem
  优点：零操作，每次会话自动获得上下文
  缺点：/compact 后丢失；只注入一次，会话中新产生的记忆不会补充
  适用：编程助手场景（会话开头需要项目上下文）
```

### 8.4 完整流程一图总结

```
         保存触发              处理管道                  检索注入
         ─────               ──────                  ──────
Mem0     每轮强制调用      →  双阶段 LLM 决策        →  按需搜索注入 prompt
Zep      每条消息调用      →  6 步流水线(13+ prompt)  →  按需搜索注入 prompt
Letta    Agent 自主判断    →  直接写入(零处理)        →  始终在 context
Engram   Agent 自主判断    →  Hash+主题键去重         →  按需 MCP 搜索
LangMem  实时+后台双通道   →  LLM 提取+合并          →  按需搜索+更新 prompt
claude-mem hooks 全自动    →  批量 AI 提炼+向量化     →  会话启动自动注入
remem    hooks 全自动      →  批量 AI 提炼+FTS索引    →  会话启动自动注入
```

### 8.5 claude-mem vs remem 保存机制对比

两者保存机制几乎一致，核心差异在处理后的存储形态：

| 环节 | **claude-mem** | **remem** |
|------|---------------|----------|
| 捕获 | PostToolUse 异步入队 | PostToolUse 异步入队 |
| 过滤 | 向量去重 | read-only 命令跳过 + 3 层 gate |
| 处理 | AI 批量提炼 | AI 批量提炼（≤15 条） |
| 存储 | SQLite + **Chroma 向量化** | SQLite + **FTS5 trigram 索引** |
| 压缩 | 无 | >100 条自动合并最旧 30 条 |
| 过期 | 无 | 90 天 TTL 自动清理 |
| 注入 | SessionStart hook | SessionStart hook |
| 搜索 | FTS5 + **向量语义** | FTS5 trigram + **LIKE 回退** |

---

## 九、remem 改进空间

1. **语义搜索缺失** — FTS5 无法理解同义词/近义词
2. **无知识图谱** — 观察之间缺少显式关系建模
3. **无可视化** — 缺少 Web Dashboard 或 TUI
4. **无 Bitemporal** — 事实推翻无显式机制
