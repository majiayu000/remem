# WorkStream 层设计文档

> 解决 remem「记录了发生什么，但不知道正在做什么、做到哪了」的问题。

## 问题诊断

remem 当前有三层数据：

| 层 | 数据 | 问题 |
|---|---|---|
| **observations** | 单次工具操作的 AI 总结 | 太碎片化，是「事件」不是「任务」 |
| **session_summaries** | 每次会话结束时的总结 | 只有回顾视角，没有前瞻视角 |
| **pending_observations** | 未处理的原始事件 | 过渡态，不是用户关心的 |

**缺失的是中间层** — 一个「项目工作流」层，回答：
1. 这个目录里我正在做什么？（活跃任务）
2. 每个任务做到什么程度了？（进度信号）
3. 之前做过什么但没做完？（暂停/阻塞的任务）

## 竞品调研

### 直接竞品（Claude Code memory 系统）

| 项目 | 核心思路 | 与 remem 差异 | 值得借鉴 |
|------|---------|-------------|---------|
| **Kiro Memory** | 自动捕获文件变更/工具调用/决策，Web Dashboard | 有 Dashboard，但也没有「任务追踪」层 | Web Dashboard、记忆衰减 |
| **Engram** | Go + SQLite FTS5 + MCP，agent 自行决定记什么 | 技术栈几乎一样，更简单 | 极简设计 |
| **MemoTrail** | 自动记录会话决策，BM25 混合搜索 | 侧重决策记录 | 自动索引历史 |
| **Memory-MCP** (yuvalsuede) | 两层：Tier1=CLAUDE.md(150行) + Tier2=state.json | 侧重事实提取 | **两层设计**：80% Tier1，20% 按需查 Tier2 |
| **Claude-Mem** | PostToolUse 全量捕获 + Chroma 向量搜索 | 多了向量搜索，架构更重 | 三层检索 |
| **CASS** | 三层认知：Episodic→Working→Procedural | 跨 agent 学习 | **三层认知架构** |
| **MCP Memory Keeper** | SQLite + 知识图谱 + 向量嵌入 | 最全面但最复杂 | 检查点机制 |

### 通用 AI Agent 记忆框架

| 项目 | 核心思路 | "正在做什么"追踪 | 值得借鉴 |
|------|---------|----------------|---------|
| **Mem0** | 事实提取 + 向量检索 | 无显式模型 | run_id 划分会话级状态 |
| **Zep/Graphiti** | 时序知识图谱，fact 有 valid_at/invalid_at | 时间窗口追踪状态变化 | **双时间线模型** |
| **Letta** (MemGPT) | Agent 自编辑核心记忆块 | **Scratchpad block 存工作状态** | **可编辑 core memory** |
| **Cognee** | 知识图谱构建 | 无 | 实体关系提取 |
| **GitHub Copilot Memory** | 带 citation 验证，28 天过期 | 无任务概念 | **Citation 验证 + 自动过期** |

### AI Coding Agent 持久化方案

| 工具 | 持久化架构 | "上次做到哪" | 值得借鉴 |
|------|----------|------------|---------|
| **Cursor** | 规则文件 + 外部 MCP | 不支持（需外部方案） | `.cursor/rules/` 目录 |
| **Aider** | Git + tree-sitter Repo Map | Git log 回溯 | **代码库就是状态** |
| **Cline** | 结构化文档（progress.md） | **显式追踪** | **Plan/Act 双模式** |
| **Continue.dev** | 向量索引 + 配置 | 不支持 | Context Providers 插件 |
| **OpenHands** | **事件溯源（Event Sourcing）** | 事件重放恢复 | **确定性重放** |
| **Windsurf** | 自动记忆 + Flow 感知 | 自动记忆推断 | IDE 动作实时追踪 |

### IDE 活动追踪工具

| 工具 | 数据模型 | 追踪粒度 | 值得借鉴 |
|------|---------|---------|---------|
| **WakaTime** | Heartbeat → Duration → Summary | 文件/分支/语言/AI行数 | **行业标准数据模型** |
| **ActivityWatch** | Bucket + Event | 窗口/应用/URL | 可扩展 watcher |
| **Code::Stats** | Pulse + XP | 语言 | 极简 |
| **GTM** | Event → Git Notes | 文件/commit | **零外部依赖** |

## 关键洞察

### 1. 没有系统原生解决这个问题
所有调研的项目——无论是 memory 框架还是 coding agent——都在做「记录发生了什么」，没有一个专门追踪「正在做什么任务、做到什么程度」。这是空白地带。

### 2. 最有价值的三个设计模式

| 模式 | 来源 | 核心思路 |
|------|------|---------|
| **可编辑状态板** | Letta/MemGPT | 一小块始终在 context 中的「当前状态」文本，agent 主动更新 |
| **时间窗口** | Zep/Graphiti | 事实有 valid_at/invalid_at，区分「正在做」和「做完了」 |
| **三层认知** | CASS | Episodic(事件) → Working(结构化) → Procedural(规则/模式) |

### 3. remem 独特优势
通过 session_summaries 的 `request` 字段，remem 能获取用户的**语义级意图**——这是 WakaTime 等纯活动追踪工具不具备的。

## 设计方案

### 架构位置

```
┌─────────────────────────────────────────────────┐
│                 Context 输出                      │
│  ┌───────────────────────────────────────────┐   │
│  │ Active WorkStreams（新增，始终显示）         │   │  ← Letta 模式
│  │  1. 实现 OAuth2 登录 [active] 还差 token 刷新│   │
│  │  2. 重构 DB 连接池   [paused]  等依赖升级   │   │
│  └───────────────────────────────────────────┘   │
│  ┌───────────────────────────────────────────┐   │
│  │ Recent Timeline（现有 observations 时间线） │   │  ← 现有逻辑
│  └───────────────────────────────────────────┘   │
└─────────────────────────────────────────────────┘

数据流：
SessionStart → 加载 active workstreams 到 context 顶部
PostToolUse  → observations 关联到当前 workstream
Stop hook    → AI 从 session summary 更新 workstream 状态
```

### 数据模型

```sql
CREATE TABLE workstreams (
    id INTEGER PRIMARY KEY,
    project TEXT NOT NULL,
    title TEXT NOT NULL,             -- AI 从 session request 提取
    description TEXT,                -- 一句话目标描述
    status TEXT DEFAULT 'active',    -- active | paused | completed | abandoned
    branch TEXT,                     -- 关联 git 分支（可选）

    -- 进度（AI 文本描述，不是百分比）
    progress_summary TEXT,           -- "完成了数据模型和 API，还差前端页面"
    next_action TEXT,                -- "添加 token 刷新逻辑"
    blockers TEXT,                   -- "等 PR review"

    -- Zep 时间窗口模式
    created_at_epoch INTEGER,
    last_active_epoch INTEGER,       -- 每次关联 session 时更新
    completed_at_epoch INTEGER,      -- status=completed 时设置

    -- 关联统计
    files_modified TEXT,             -- JSON array，累积
    sessions_count INTEGER DEFAULT 0,
    observations_count INTEGER DEFAULT 0
);

-- WorkStream <-> Session 多对多
CREATE TABLE workstream_sessions (
    workstream_id INTEGER REFERENCES workstreams(id),
    memory_session_id TEXT,
    PRIMARY KEY (workstream_id, memory_session_id)
);
```

### 生命周期

```
1. 创建：Stop hook → AI 从 session_summary.request 提取任务
   - 与已有 active workstream 匹配（FTS5 语义匹配）
   - 匹配到 → 更新已有 workstream
   - 没匹配 → 创建新 workstream

2. 更新：Stop hook → AI 从 completed/next_steps 更新进度
   - progress_summary: 从 completed 字段提取
   - next_action: 从 next_steps 字段提取
   - files_modified: 从 observations 聚合

3. 关闭：
   - AI 判断 session 完成了该任务 → completed
   - 超过 7 天无活动 → paused（自动）
   - 超过 30 天无活动 → abandoned（自动）

4. 恢复：新 session 的 request 匹配到 paused workstream → 重新激活
```

### Context 输出

SessionStart 时在时间线前面插入：

```markdown
## Active WorkStreams

| # | Task | Status | Last Active | Next Action |
|---|------|--------|-------------|-------------|
| WS-1 | 实现 OAuth2 登录 | active | 2h ago | 添加 token 刷新逻辑 |
| WS-3 | 重构 DB 连接池 | paused (5d) | 5d ago | 等依赖升级后继续 |
```

### 新增 MCP 工具

```
workstreams(project?, status?) → 列出工作流
update_workstream(id, status?, next_action?, blockers?) → 手动更新
```

### AI Prompt 扩展

在现有 summary prompt 中加入 workstream 相关字段：

```xml
<workstream_update>
  <match_existing>[title of existing workstream if continuing, or "new"]</match_existing>
  <title>[workstream title]</title>
  <status>[active|completed]</status>
  <progress_summary>[what's done so far]</progress_summary>
  <next_action>[what to do next]</next_action>
</workstream_update>
```

不需要额外 AI 调用——复用现有 session summary 的 AI 调用。

## 实现优先级

| 优先级 | 任务 | 涉及文件 | 工作量 |
|--------|------|---------|--------|
| **P0** | `workstreams` 表 + schema migration | db.rs | 小 |
| **P0** | summary prompt 扩展 + 解析 workstream 字段 | prompts/summary.txt, summarize.rs | 中 |
| **P0** | Stop hook 中自动创建/更新 workstream | summarize.rs, db.rs | 中 |
| **P1** | Context 输出「Active WorkStreams」段落 | context.rs | 小 |
| **P2** | MCP 工具 `workstreams` / `update_workstream` | mcp.rs | 中 |
| **P3** | 自动暂停/关闭过期 workstream（cleanup） | summarize.rs | 小 |
| **P3** | FTS5 索引 workstream title 用于匹配 | db.rs | 小 |

## 设计决策记录

| 决策 | 选择 | 理由 |
|------|------|------|
| 进度表示方式 | 文本描述，不是百分比 | AI 猜百分比不准；文本描述（"做了 X，还差 Y"）更有用 |
| 创建时机 | Stop hook（复用 summary AI 调用） | 不增加额外 AI 调用成本 |
| 匹配策略 | FTS5 语义匹配已有 workstream | 避免重复创建；允许跨 session 关联 |
| 关联粒度 | 1 workstream : N sessions | 一个任务可能跨多个 session |
| 自动关闭 | 7d→paused, 30d→abandoned | 避免 stale workstream 堆积 |
| 存储位置 | 同一个 remem.db | 不引入新的存储依赖 |

## 参考资料

### 直接竞品
- [Kiro Memory](https://github.com/Auriti-Labs/kiro-memory) — Web Dashboard + 向量搜索
- [Engram](https://github.com/Gentleman-Programming/engram) — Go + SQLite FTS5 + MCP
- [MemoTrail](https://github.com/HalilHopa-Datatent/memotrail) — 自动会话决策记录
- [Memory-MCP](https://github.com/yuvalsuede/memory-mcp) — 两层记忆架构
- [Claude-Mem](https://github.com/thedotmack/claude-mem) — PostToolUse 全量捕获 + Chroma
- [CASS](https://github.com/Dicklesworthstone/cass_memory_system) — 三层认知架构
- [MCP Memory Keeper](https://github.com/mkreyman/mcp-memory-keeper) — 知识图谱 + 多代理

### 通用 AI Agent 记忆框架
- [Mem0](https://github.com/mem0ai/mem0) — 事实提取 + 向量检索
- [Zep/Graphiti](https://github.com/getzep/graphiti) — 时序知识图谱
- [Letta](https://github.com/letta-ai/letta) — Agent 自编辑核心记忆
- [OpenContext](https://github.com/0xranx/OpenContext) — 个人知识库 MCP

### AI Coding Agent 持久化
- [Cline Memory Bank](https://docs.cline.bot/features/cline-rules) — progress.md 显式追踪
- [OpenHands](https://github.com/OpenHands/OpenHands) — 事件溯源架构
- [Aider Repo Map](https://aider.chat/docs/repomap.html) — tree-sitter 代码库理解
- [CCPM](https://github.com/automazeio/ccpm) — GitHub Issues + Git Worktrees

### 活动追踪
- [WakaTime Heartbeat API](https://wakatime.com/developers) — 行业标准数据模型
- [ActivityWatch](https://docs.activitywatch.net/) — 开源 Bucket+Event 模型
- [GTM](https://github.com/git-time-metric/gtm) — Git Notes 零依赖方案

### GitHub Copilot Memory
- [Building Agentic Memory for Copilot](https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/) — Citation 验证 + 28 天过期
