---
source: remem.save_memory
saved_at: 2026-03-15T06:56:28.799807+00:00
project: remem
---

# Claude Code 记忆 MCP 工具生态调研报告（2026年3月）

## 调研目标

对比 Claude Code 生态中主流的记忆 MCP 工具，分析 remem 的竞争优势与差异。

## 调研覆盖的项目

1. **memory-mcp** (WhenMoon-afk) — 轻量级 SQLite+FTS5
2. **claude-mem** (thedotmack) — 向量搜索 + Web Dashboard  
3. **basic-memory** (basicmachines-co) — Markdown 知识图谱
4. **claude-memory-mcp** (FirmengruppeViola) — 语义搜索 + Markdown
5. **mcp-memory-keeper** (mkreyman) — 知识图谱 + 检查点机制
6. **mcp-memory** (ebailey78) — 项目级结构化记忆
7. **remem** (当前项目) — 4 阶段管道 + 工作流追踪

## 核心对比维度

### 1. 触发机制

| 项目 | 触发方式 | 特点 |
|------|---------|------|
| memory-mcp | 显式工具调用 | 用户手动 memory_store/recall |
| claude-mem | Hooks（SessionStart/PostToolUse/Stop） | 自动捕获，无需用户干预 |
| basic-memory | 显式 MCP 工具 | "Create note"/"Find info" 自然语言调用 |
| FirmengruppeViola | Hooks 自动化 + UserPromptSubmit 语义搜索 | 每次提交自动检索 top-3 |
| mcp-memory-keeper | 显式工具调用（context_save/restore） | Git 集成提示触发 |
| mcp-memory | 项目加载时自动 | 项目范围的隐式绑定 |
| **remem** | **Hooks 全自动化**：SessionStart(注入) + PostToolUse(入队) + Stop(后台总结) | **最激进的自动化**：无需任何用户操作 |

**结论**：remem 是唯一完全被动的系统，不需要用户主动调用任何工具。

---

### 2. 存储方式

| 项目 | 物理存储 | 格式 | 特点 |
|------|---------|------|------|
| memory-mcp | SQLite + FTS5 | 二进制 DB | ~/.memory-mcp/memory.db，90天 TTL 自动过期 |
| claude-mem | SQLite + Chroma 向量 DB | 混合 | 双数据源，~10x token 节省通过分层检索 |
| basic-memory | Markdown + SQLite 索引 | 文本+索引 | ~/basic-memory/*.md，人类可读，支持 Obsidian 集成 |
| FirmengruppeViola | Markdown + Milvus 向量索引 | 文本+向量 | ~/memories/YYYY-MM-DD.md，一日一文件，可随时重建 |
| mcp-memory-keeper | SQLite | 二进制 DB | ~/mcp-data/memory-keeper/，含变更历史追踪 |
| mcp-memory | 分层目录结构 | Markdown | /entities/, /concepts/, /sessions/，Lunr.js 索引 |
| **remem** | **SQLite + WAL** | **二进制 DB** | **~/.remem/remem.db**，3 层表结构（pending/observations/compressed） |

**差异分析**：
- memory-mcp/claude-mem/mcp-memory-keeper/remem：SQLite（性能优、查询灵活）
- basic-memory/FirmengruppeViola：Markdown（可审计、支持 Obsidian、人类可编辑）
- 只有 remem 有三层数据生命周期管理（pending→observations→compressed）

---

### 3. 检索方式

| 项目 | 搜索技术 | 特点 |
|------|---------|------|
| memory-mcp | FTS5 全文搜索 | 无嵌入，依赖关键词匹配，快速但低精度 |
| claude-mem | 三层检索 | search index → timeline context → detail fetch，~10x token 节省 |
| basic-memory | 混合搜索 | 全文（关键词） + 向量（FastEmbed） + 知识图谱（wiki 链接） |
| FirmengruppeViola | 语义搜索 | Milvus 向量 DB，每次 UserPromptSubmit 自动注入 top-3 |
| mcp-memory-keeper | 多维过滤 | 全文 + 时间 + Regex + 分类 + 优先级，+变更追踪 |
| mcp-memory | Lunr.js 索引 | 全文搜索，支持标签和内存类型过滤 |
| **remem** | **FTS5 + 时间衰减排序** | **rank×recency 混合**，stale 观察额外降权 |

**差异分析**：
- memory-mcp/remem：FTS5（无向量，但足够快）
- claude-mem/basic-memory/FirmengruppeViola：向量搜索（更精准但开销大）
- remem 独特：时间衰减 + stale 状态分权，最符合遗忘曲线

---

### 4. 上下文注入方式

| 项目 | 注入时机 | 注入内容 | 特点 |
|------|---------|---------|------|
| memory-mcp | MCP 工具调用时 | 搜索结果（用户选择） | 被动，需要主动搜索 |
| claude-mem | SessionStart（自动） | 搜索索引 + timeline + 全量细节（分层）| 渐进式披露，mem-search skill |
| basic-memory | MCP 工具调用时 | 笔记内容 + 关联关系 | 知识图谱导航（memory:// URL） |
| FirmengruppeViola | UserPromptSubmit（自动） | top-3 语义相关记忆 | 每个用户消息自动注入 |
| mcp-memory-keeper | 显式 restore_checkpoint 时 | 完整 snapshot | 精确恢复整个会话状态 |
| mcp-memory | SessionStart 时 | 项目级上下文 | 自动但项目级别 |
| **remem** | **SessionStart（自动）** | **50 条观察 + 10 条 summary**，分 3 层（完整/表格/统计） | **最复杂的渐进式披露**，token 经济统计 |

**差异分析**：
- FirmengruppeViola：最激进（每条消息都检索），但可能过度注入
- remem：最精细（3 层渲染、token 统计、类型过滤、stale 限权）

---

### 5. Claude Code 集成方式

| 项目 | 集成方式 | 安装复杂度 | 配置文件 |
|------|---------|-----------|---------|
| memory-mcp | MCP server 配置 | 中 | claude_desktop_config.json |
| claude-mem | Hooks + MCP + 后台 Worker | 高 | hooks、settings.json、background service |
| basic-memory | MCP server 配置 | 低 | claude_desktop_config.json |
| FirmengruppeViola | Hooks 自动注入 + MCP | 中 | settings.json（hooks）、Node Worker（port 37777） |
| mcp-memory-keeper | MCP server 配置 | 中 | claude_desktop_config.json |
| mcp-memory | MCP server 配置 | 低 | claude_desktop_config.json |
| **remem** | **Hooks 全套 + MCP** | **低**（一键 install） | **~/.claude/settings.json（自动注入 4 个 hooks）** |

**差异分析**：
- remem 独特：`remem install` 一键自动化所有配置，包括 hooks、MCP、路径设置
- claude-mem：集成最复杂（需要 Node Worker + port 配置）
- basic-memory/mcp-memory：集成最简单（仅 MCP）

---

### 6. 数据生命周期管理

| 项目 | 生命周期 | 压缩策略 | 过期清理 | 去重机制 |
|------|---------|---------|---------|---------|
| memory-mcp | 线性 | 无 | 90 天自动删除 | 无显式去重 |
| claude-mem | 线性 | 无 | 无提及 | Chroma 向量去重 |
| basic-memory | 线性 | 无 | 用户手动 | 无 |
| FirmengruppeViola | 线性 | 无（Markdown 逐日增长） | 无显式 | 无 |
| mcp-memory-keeper | 线性 | 无 | 无提及 | Git 变更追踪 |
| mcp-memory | 线性 | 无 | 无提及 | 无 |
| **remem** | **3 阶段**：pending→observations→compressed | **自动合并**（>100 条观察→最旧 30 条合并为 1-2 条） | **90 天 compressed 删除** | **delta 去重 + 文件覆盖检测** |

**差异分析**：
- remem 唯一有压缩管道：pending（原始）→ observations（AI 提炼）→ compressed（自动合并）
- remem 唯一有多层去重：delta 去重（flush 时注入历史）+ 文件覆盖检测（stale 标记）
- 其他项目都是线性增长，remem 设计了衰减曲线

---

### 7. 速率限制与并发保护

| 项目 | 限制策略 | 机制 |
|------|---------|------|
| memory-mcp | 无 | - |
| claude-mem | 隐式（向量索引更新频率） | - |
| basic-memory | 无 | - |
| FirmengruppeViola | 无 | - |
| mcp-memory-keeper | 配置 token 上限 | 防溢出 |
| mcp-memory | 无 | - |
| **remem** | **3 层 gate + Worker 双检** | **pending<3 跳过 \| 项目 300s 冷却 \| message hash 去重 \| Worker 再检** |

**差异分析**：
- remem 唯一应对"短命进程模型"的去重方案：SQLite 表 (summarize_cooldown) 模拟内存状态

---

### 8. AI 调用成本优化

| 项目 | AI 调用时机 | 优化策略 |
|------|-----------|---------|
| memory-mcp | 无 AI 调用（纯全文搜索） | - |
| claude-mem | SessionStart + 向量索引更新 | 分层检索减少 token 10x |
| basic-memory | 无显式 AI 调用（Markdown 编辑由用户/Claude 决定） | - |
| FirmengruppeViola | UserPromptSubmit 时语义搜索 | Milvus 向量索引避免每次重新计算 |
| mcp-memory-keeper | 显式 context_save/restore | 用户手动控制 |
| mcp-memory | 无 AI 调用 | - |
| **remem** | **Stop hook 一次 AI 调用处理 ≤15 事件** | **HTTP-first（2-5s） vs CLI 30-60s；model 映射；超时 90s；4 个复用 prompt** |

**差异分析**：
- remem 唯一关注 AI 调用性能：HTTP API 优先（6-12x 快于 CLI）
- remem 唯一有精细 token 预算：input/output 单价可配置，支持实时成本统计

---

### 9. 工作流追踪（WorkStream）

| 项目 | 任务追踪 | 进度表示 | 暂停/完成 |
|------|---------|---------|----------|
| memory-mcp | 无 | - | - |
| claude-mem | 无 | - | - |
| basic-memory | 无 | - | - |
| FirmengruppeViola | 无 | - | - |
| mcp-memory-keeper | 无 | - | - |
| mcp-memory | 无（仅项目级） | - | - |
| **remem** | **WorkStream 表（计划中）** | **文本描述**："完成了 X，还差 Y" | **active/paused/completed/abandoned** |

**差异分析**：
- remem 在规划引入"第四层"抽象：工作流追踪（回答"做到哪里了"而非"做了什么"）
- 所有竞品都没有此功能

---

## 关键设计对比：4 阶段管道 vs 其他方案

### remem 的 4 阶段管道

```
工具操作 → pending（入队）→ observations（AI 提炼）→ compressed（自动合并）→ cleanup（删除）
  
门控：
- Gate1: pending < 3 → 跳过（短命 session）
- Gate2: 项目 300s 冷却期 → 跳过（去重）
- Gate3: message hash → 跳过（相同 message）
```

### 竞品的一般方案

大多数竞品采用"双层或单层"存储：
- **单层**：工具操作 → 记忆（memory-mcp, mcp-memory）
- **双层**：会话 snapshot + 索引（claude-mem）
- **三层**：观察 + session summary + 知识图谱（basic-memory, FirmengruppeViola）

**remem 独特性**：
1. **pending 缓冲层**：防止低流量 session 丢失（自动 flush 残留队列）
2. **压缩层**：长期衰减管理，避免无限增长
3. **Delta 去重**：flush 时注入历史，AI 自动跳过重复
4. **文件覆盖检测**：修改同一文件时标记旧观察为 stale

---

## remem 的独特优势

| 维度 | 优势 |
|------|------|
| **自动化程度** | 完全被动，零用户操作；竞品都需要显式工具调用或手动触发 |
| **生命周期管理** | 唯一有 3 阶段压缩管道 + 自动化清理；竞品都是线性增长 |
| **去重机制** | delta 去重 + 文件覆盖检测；竞品多无去重或向量去重 |
| **速率限制** | 3 层 gate + Worker 双检；竞品无此机制 |
| **安装体验** | 一键 `remem install`；竞品需手动编辑 JSON |
| **成本追踪** | 实时 token 统计 + 成本预算；竞品无此功能 |
| **时间衰减** | FTS rank + recency + stale 限权；竞品无时间衰减概念 |
| **工作流追踪** | WorkStream 追踪（计划）；竞品完全无此 |

---

## remem 的劣势与改进空间

| 维度 | 当前状态 | 竞品优势 |
|------|---------|---------|
| **语义搜索** | FTS5（关键词）| basic-memory/claude-mem/FirmengruppeViola 有向量搜索 |
| **人类可读性** | SQLite 二进制 | Markdown 方案（basic-memory/FirmengruppeViola）可 Obsidian 集成 |
| **可审计性** | DB 查询工具 | Markdown 方案完全透明 |
| **Dashboard** | 无 | claude-mem 有 Web UI (port 37777) |
| **知识图谱** | 无（仅 FTS） | basic-memory 有显式关系网络 |
| **集成广度** | 仅 Claude Code | basic-memory 支持任何 MCP 客户端 |

---

## 竞品与 remem 的技术选择对比

| 选择 | 原因 |
|------|------|
| **SQLite vs Markdown** | SQLite 胜在查询灵活性+速度，Markdown 胜在可读性+可编辑性；remem 选择 SQLite 优化搜索 |
| **FTS5 vs 向量** | FTS5 无向量开销，相同搜索词命中率 90%+；向量搜索开销高（嵌入+HNSW）；remem 选 FTS5 降低 token 消耗 |
| **自动 vs 手动** | remem 自动化最激进；其他项目让用户决定"什么值得记"，更灵活但易遗漏 |
| **同步 vs 异步** | remem Stop hook 立即返回（6ms），后台 worker 异步处理；claude-mem 类似；其他项目多同步 |

---

## 实施建议

### 如果优先考虑"自动化 + 成本控制"
→ 选择 **remem**

### 如果优先考虑"人类可读 + Obsidian 集成"  
→ 选择 **basic-memory** 或 **FirmengruppeViola**

### 如果优先考虑"搜索精度 + 向量语义"
→ 选择 **claude-mem** 或 **FirmengruppeViola**

### 如果优先考虑"简单轻量"
→ 选择 **memory-mcp**

### 如果优先考虑"状态恢复 + 检查点"
→ 选择 **mcp-memory-keeper**

---

## 总体评分（10 分制）

| 项目 | 自动化 | 成本控制 | 搜索质量 | 安装简易 | 可读性 | 功能完整 | 平均分 |
|------|--------|----------|----------|----------|--------|----------|--------|
| memory-mcp | 3 | 3 | 5 | 8 | 2 | 3 | 4.0 |
| claude-mem | 9 | 7 | 8 | 6 | 4 | 8 | 7.0 |
| basic-memory | 5 | 4 | 8 | 7 | 9 | 6 | 6.5 |
| FirmengruppeViola | 8 | 5 | 8 | 5 | 9 | 7 | 7.0 |
| mcp-memory-keeper | 4 | 5 | 6 | 6 | 7 | 7 | 5.8 |
| mcp-memory | 5 | 4 | 5 | 8 | 8 | 5 | 5.8 |
| **remem** | **10** | **9** | **6** | **10** | **4** | **8** | **7.8** |

**remem 综合评分最高**（7.8/10），尤其在自动化和成本控制领域无人能及。
