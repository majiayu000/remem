# Cursor 上下文记忆机制深度调研

> 调研日期：2026-03-16
> 目标：理解 Cursor 如何提取、保存、更新和检索上下文记忆

## 目录

1. [核心架构](#核心架构)
2. [如何提取](#如何提取)
3. [提取什么](#提取什么)
4. [如何保存](#如何保存)
5. [如何更新](#如何更新)
6. [关键技术实现](#关键技术实现)
7. [对 remem 的启示](#对-remem-的启示)

---

## 核心架构

Cursor 的上下文记忆系统由三层组成：

### 1. VS Code Fork + AI-Native Extensions
- 基于 VS Code 深度定制，不是简单的插件
- 集成 AI 能力到编辑器核心，而非外挂层

### 2. AI Model Orchestration Layer
- 支持多模型并行（GPT-4、Claude、Gemini）
- Composer 模式可同时运行 8 个 agent
- 使用 speculative decoding 优化补全速度

### 3. Context-Aware Processing Engine
- 实时代码分析
- 动态上下文发现（Dynamic Context Discovery）
- 语义搜索 + 向量检索

---

## 如何提取

### 1. .cursorrules 文件加载

**加载时机**：
- 项目打开时自动加载
- 每次 AI 请求时注入到 system prompt
- **不支持热重载** — 修改后需要重启 Cursor 或重新打开项目

**层级结构**：
```
.cursorrules              # 项目根目录（优先级最高）
.cursor/rules/*.md        # 多文件规则（支持路径模式匹配）
~/.cursor/rules/          # 全局规则（所有项目共享）
```

**作用范围**：
- Chat 模式：全量注入
- Cmd+K 模式：选择性注入（token 预算更紧张）
- Composer 模式：跨文件上下文时全量注入

### 2. @-mentions 上下文注入

| 命令 | 触发时机 | 上下文来源 |
|------|---------|-----------|
| `@Codebase` | 手动 / 自动 | 向量数据库语义检索 |
| `@Docs` | 手动 | 外部文档爬取 + 索引 |
| `@Web` | 手动 | 实时搜索 + 页面抓取 |
| `@File` | 手动 | 直接读取文件内容 |
| `@Folder` | 手动 | 目录下所有文件 |
| `@Git` | 手动（已废弃） | 现在由 Agent 自行调用 git 命令 |

**@Codebase 工作流程**：
1. 用户查询 → 生成查询 embedding
2. 向量数据库检索 top-k 相关代码块
3. 按相关性排序注入到 context window
4. 动态调整 token 预算（Chat: ~20k, Cmd+K: ~10k）

**@Docs 工作流程**：
1. 用户提供文档 URL
2. Cursor 爬取页面内容
3. 转换为 Markdown 格式
4. 计算 embedding 并存储
5. 后续查询时语义检索相关片段

### 3. Composer 多文件上下文管理

**Composer 模式特点**：
- 自动识别跨文件依赖
- 维护文件间引用关系
- 并行编辑多个文件（最多 50+ 文件）
- 使用 git worktrees 隔离并行任务

**上下文发现策略**：
- **静态分析**：AST 解析 import/export 关系
- **语义搜索**：向量检索相关代码
- **LSP 集成**：利用 Language Server Protocol 获取符号定义
- **Git 历史**：分析最近修改的文件

### 4. 自动上下文检索

**触发条件**：
- 用户查询包含代码相关关键词
- Cmd+Enter 快捷键（强制触发 @Codebase）
- Composer 模式下的跨文件操作
- 补全请求（Tab 触发）

**排序算法**：
- 语义相似度（embedding cosine similarity）
- 文件最近修改时间
- 文件打开频率
- 当前光标位置的上下文

**Token 预算管理**：
- 动态调整上下文窗口大小
- 优先级：当前文件 > 最近编辑 > 语义相关 > 项目结构
- 超出预算时截断低优先级内容

---

## 提取什么

### 1. Codebase Indexing 范围

**全量索引**：
- 项目打开时自动触发
- 使用 Merkle Tree 检测变更
- 只索引文本文件（排除 binary、node_modules、.git）

**增量索引**：
- 文件保存时触发
- 只重新索引变更的文件
- Merkle Tree 根哈希变化 → 定位变更节点

**索引内容**：
- 文件路径（相对路径）
- 代码块（通过 Tree-sitter 分割）
- 起止行号
- Embedding 向量

### 2. 代码语义理解

**Tree-sitter 解析**：
- 将代码解析为 AST（Abstract Syntax Tree）
- 按语法单元分块（函数、类、方法）
- 保留语义完整性（不会在表达式中间切断）

**分块策略**：
```
1. 使用 Tree-sitter 解析代码
2. 提取顶层节点（函数、类、接口）
3. 如果节点超过 token 限制（~8k）：
   - 递归分割子节点
   - 合并相邻小节点
4. 为每个块生成 embedding
5. 存储：embedding + 文件路径 + 行号范围
```

**支持的语言**：
- 所有 Tree-sitter 支持的语言（50+ 种）
- 包括 Rust、TypeScript、Python、Go、Java 等

### 3. 文档索引

**README 和注释**：
- 自动提取项目 README.md
- 解析代码注释（JSDoc、docstring、/// 等）
- 作为高优先级上下文

**外部文档**：
- 通过 @Docs 手动添加
- 支持多页面爬取（sitemap 自动发现）
- 存储在远程向量数据库（turbopuffer）

### 4. 用户偏好记忆

**Memories 功能**（v0.51+ 引入）：
- 用户可以手动保存"记忆"
- 存储在 Cursor 云端（不是本地）
- 自动索引并在相关上下文时检索
- 支持项目级和全局级记忆

**代码风格学习**：
- **不自动学习** — Cursor 不会自动推断你的代码风格
- 需要通过 .cursorrules 显式声明
- 可以引用 ESLint/Prettier 配置

---

## 如何保存

### 1. 本地索引存储

**向量数据库**：
- 使用 **turbopuffer**（Cursor 自研）
- 基于对象存储（S3）而非内存
- 成本降低 20x，支持 100B+ 向量

**存储内容**：
```json
{
  "embedding": [0.123, -0.456, ...],  // 向量
  "file_path": "src/main.rs",         // 混淆后的相对路径
  "start_line": 10,
  "end_line": 25,
  "chunk_hash": "abc123..."           // 用于去重
}
```

**隐私保护**：
- **代码不上传** — 只上传 embedding 和元数据
- 文件路径混淆（obfuscated relative path）
- 本地计算 embedding，远程只存储向量

### 2. .cursorrules 持久化

**存储位置**：
```
项目级：<project>/.cursorrules
       <project>/.cursor/rules/*.md
全局级：~/.cursor/rules/
```

**版本控制**：
- .cursorrules 应该提交到 git
- 团队共享规则
- .cursor/rules/ 可以 gitignore（个人规则）

### 3. 会话历史保存

**Chat 历史**：
- 存储在本地：`~/AppData/Roaming/Cursor/User/workspaceStorage/<workspace-id>/state.vscdb`
- 基于 workspace 隔离
- **不跨设备同步**
- 关闭项目后保留

**Composer 历史**：
- 每个 Composer 会话独立
- 保存在 workspace storage
- 包含文件变更记录

### 4. Memories 存储

**存储位置**：
- **云端存储**（Cursor 服务器）
- 不是本地文件

**索引机制**：
- 云端计算 embedding
- 查询时语义检索
- 按相关性注入到上下文

**隐私争议**：
- 用户担心数据上传
- Cursor 团队解释：需要云端索引才能高效检索大量记忆
- 可以选择不使用 Memories 功能

---

## 如何更新

### 1. 文件变更的增量索引

**Merkle Tree 机制**：
```
1. 项目初始化时构建 Merkle Tree
   - 叶子节点：文件内容哈希
   - 中间节点：子节点哈希的哈希
   - 根节点：整个项目的哈希

2. 文件变更时：
   - 重新计算变更文件的哈希
   - 向上传播更新父节点哈希
   - 根哈希变化 → 触发增量索引

3. 同步到服务器：
   - 比较本地和远程根哈希
   - 只上传变更的子树
   - 减少 90% 的上传量
```

**触发时机**：
- 文件保存（自动）
- 项目重新打开（全量检查）
- 手动触发索引（设置中）

### 2. .cursorrules 热重载

**当前状态**：
- **不支持热重载**
- 修改后需要：
  - 重启 Cursor，或
  - 关闭并重新打开项目

**社区解决方案**：
- 使用 Cursor Agent 自动更新 .cursorrules
- 通过 MCP 服务器动态注入规则

### 3. 上下文窗口动态调整

**Token 预算策略**：
```
Chat 模式：~20k tokens
├─ System Prompt (Rules): 2-5k
├─ 当前文件: 3-8k
├─ @-mentions: 5-10k
└─ 历史对话: 2-5k

Cmd+K 模式：~10k tokens
├─ System Prompt: 1-2k
├─ 当前选中代码: 2-5k
└─ 相关上下文: 3-5k
```

**动态调整算法**：
1. 计算必需上下文（当前文件、用户查询）
2. 剩余 token 预算分配给可选上下文
3. 按优先级排序：
   - 用户显式 @-mention
   - 语义相关代码
   - 最近编辑文件
   - 项目结构信息
4. 超出预算时截断低优先级内容

**Dynamic Context Discovery**（Cursor 2.0）：
- 不再一次性加载所有上下文
- 按需检索（lazy loading）
- 减少 46.9% token 使用
- 提升性能和准确性

### 4. 记忆过期策略

**Codebase Index**：
- 文件删除 → 立即从索引移除
- 文件重命名 → 旧路径失效，新路径重新索引
- 无时间过期（只要文件存在就保留）

**Memories**：
- **无自动过期** — 用户手动删除
- 可能导致过时信息累积
- 社区建议定期清理

**Chat History**：
- 基于 workspace 隔离
- 删除 workspace → 历史丢失
- 无跨项目记忆

---

## 关键技术实现

### 1. turbopuffer 向量数据库

**架构特点**：
- **对象存储优先**（S3）而非内存/SSD
- 成本：$0.10/GB/月（传统方案 $2+/GB/月）
- 支持 100B+ 向量规模

**性能优化**：
- SPFresh 算法（近似最近邻搜索）
- Recall@10 > 90-95%
- 查询延迟 < 100ms

**为什么选择对象存储**：
- 向量数据库的瓶颈是存储成本，不是计算
- 大部分查询只访问少量向量
- 对象存储 + 智能缓存 = 低成本 + 高性能

### 2. Tree-sitter 语法解析

**为什么用 Tree-sitter**：
- 增量解析（只重新解析变更部分）
- 容错性强（语法错误不会导致解析失败）
- 支持 50+ 语言
- 比 LSP 更轻量（不需要启动 language server）

**与 LSP 的配合**：
- Tree-sitter：语法级别（分块、高亮）
- LSP：语义级别（跳转定义、重命名）
- Cursor 同时使用两者

**分块示例**（Rust）：
```rust
// 原始代码
fn main() {
    let x = 42;
    println!("{}", x);
}

// Tree-sitter 解析后
[
  {
    "type": "function_item",
    "start_line": 1,
    "end_line": 4,
    "text": "fn main() { ... }"
  }
]
```

### 3. Merkle Tree 增量索引

**数据结构**：
```
Root Hash: abc123
├─ src/ (hash: def456)
│  ├─ main.rs (hash: 789abc)
│  └─ lib.rs (hash: def012)
└─ tests/ (hash: 345678)
   └─ test.rs (hash: 901234)
```

**变更检测**：
```
1. 修改 src/main.rs
2. 重新计算 main.rs 哈希 → 789abc → 789xyz
3. 向上传播：
   - src/ 哈希变化 → def456 → def999
   - Root 哈希变化 → abc123 → abc888
4. 只重新索引 src/main.rs
```

**优势**：
- O(log n) 变更检测
- 只上传变更的子树
- 支持大型项目（100k+ 文件）

### 4. Speculative Decoding 补全优化

**原理**：
- 使用小模型（draft model）快速生成候选 token
- 大模型（target model）并行验证
- 接受正确的 token，拒绝错误的

**Cursor 的实现**：
- Draft model：自研 sparse model
- 利用现有代码预测下一个 token
- 2-3x 补全速度提升

**为什么有效**：
- 代码补全高度可预测（变量名、函数调用）
- 小模型足以处理常见模式
- 大模型只需验证，不需要生成

### 5. Dynamic Context Discovery

**传统方法问题**：
- 一次性加载所有可能相关的上下文
- 浪费 token 预算
- 降低准确性（噪音太多）

**Cursor 2.0 方案**：
```
1. 初始查询 → 生成初步计划
2. 识别需要的上下文类型
3. 按需检索：
   - 需要 API 文档 → @Docs
   - 需要相关代码 → @Codebase
   - 需要外部信息 → @Web
4. 动态调整上下文窗口
5. 执行任务
```

**效果**：
- Token 使用减少 46.9%
- 准确性提升（更少噪音）
- 延迟降低（更少无用检索）

---

## 对 remem 的启示

### 1. 不要依赖 Claude 主动调用

**Cursor 的教训**：
- Notepads 功能被废弃 → 用户不会主动 @notepad
- Memories 功能使用率低 → 用户懒得手动保存
- 最有效的是**自动捕获** + 手动补充

**remem 应该**：
- ✅ 自动捕获所有对话（主力）
- ✅ 提供 save_memory 工具（补充）
- ❌ 不要指望 Claude 主动调用 save_memory

### 2. 向量检索是核心

**Cursor 的实践**：
- @Codebase 是最常用的功能
- 语义搜索比关键词搜索更有效
- 需要高质量的 embedding 模型

**remem 应该**：
- ✅ 投资向量数据库（SQLite + vec0？）
- ✅ 使用好的 embedding 模型（OpenAI/Voyage）
- ✅ 优化检索算法（混合搜索：语义 + 关键词）

### 3. 分块策略很重要

**Cursor 的方案**：
- 使用 Tree-sitter 按语法单元分块
- 保留语义完整性
- 动态调整块大小

**remem 应该**：
- ✅ 按对话轮次分块（自然边界）
- ✅ 提取关键信息（facts、decisions、code snippets）
- ✅ 避免切断上下文（保留前后关联）

### 4. 增量更新是必需的

**Cursor 的方案**：
- Merkle Tree 检测变更
- 只重新索引变更部分
- 支持大规模项目

**remem 应该**：
- ✅ 每次对话后增量更新
- ✅ 避免全量重建索引
- ✅ 使用时间戳或哈希检测变更

### 5. 多层次记忆

**Cursor 的层次**：
```
全局规则 (~/.cursor/rules/)
  ↓
项目规则 (.cursorrules)
  ↓
Memories（云端）
  ↓
Chat History（本地）
  ↓
Codebase Index（向量数据库）
```

**remem 应该**：
```
全局记忆（跨项目）
  ↓
项目记忆（当前项目）
  ↓
会话记忆（当前对话）
  ↓
工作流记忆（workstreams）
```

### 6. 隐私和成本的权衡

**Cursor 的选择**：
- Codebase Index：只上传 embedding（隐私优先）
- Memories：上传内容到云端（功能优先）
- 用户可以选择不用 Memories

**remem 应该**：
- ✅ 本地优先（SQLite + 本地 embedding）
- ✅ 可选云端同步（高级功能）
- ✅ 透明告知用户数据存储位置

### 7. 不要过度设计

**Cursor 废弃的功能**：
- Notepads → 被 Rules/Memories 替代
- @Git → Agent 自己调用 git 命令更灵活

**remem 应该**：
- ✅ 先做好基础：自动捕获 + 语义检索
- ✅ 避免过早优化（知识图谱、多模态）
- ✅ 等基础稳定后再加高级功能

### 8. 用户体验优先

**Cursor 的成功因素**：
- 零配置（打开项目就能用）
- 自动索引（不需要手动触发）
- 快速响应（< 100ms 检索延迟）

**remem 应该**：
- ✅ 零配置启动（自动检测项目）
- ✅ 后台自动捕获（不打断用户）
- ✅ 快速检索（< 200ms 响应）

---

## 参考资料

### 官方文档
- [Cursor – Codebase Indexing](https://docs.cursor.com/context/codebase-indexing)
- [Cursor – Rules](https://docs.cursor.com/context/rules)
- [Cursor – Working with Documentation](https://docs.cursor.com/en/guides/advanced/working-with-documentation)
- [Securely indexing large codebases · Cursor](https://cursor.com/blog/secure-codebase-indexing)

### 技术分析
- [How Cursor Actually Indexes Your Codebase](https://towardsdatascience.com/how-cursor-actually-indexes-your-codebase/)
- [Context Management Strategies for Cursor](https://blog.datalakehouse.help/posts/2026-03-context-cursor/)
- [Cursor: Dynamic Context Discovery for Production Coding Agents](https://www.zenml.io/llmops-database/dynamic-context-discovery-for-production-coding-agents)
- [How Cursor Indexes Codebases Fast](https://read.engineerscodex.com/p/how-cursor-indexes-codebases-fast)
- [Architecture and Engineering](https://theaiengineer.substack.com/p/how-cursor-actually-works)

### 向量数据库
- [Cursor scales code retrieval to 100B+ vectors with turbopuffer](https://turbopuffer.com/customers/cursor)
- [TurboPuffer: Object Storage-First Vector Database Architecture](https://jxnl.co/writing/2025/09/11/turbopuffer-object-storage-first-vector-database-architecture/)

### 社区讨论
- [How Cursor Context works as of 0.45.7](https://forum.cursor.com/t/how-cursor-context-works-as-of-0-45-7/47177)
- [Does Cursor AI Track Memory Across Conversations?](https://www.blockchain-council.org/ai/cursor-ai-track-memory-across-conversations/)
- [Memory Bank feature for your Cursor](https://forum.cursor.com/t/memory-bank-feature-for-your-cursor/71979)
- [0.51: "Memories" feature](https://forum.cursor.com/t/0-51-memories-feature/98509)
- [Best way to provide context: Rules vs. Memories](https://forum.cursor.com/t/best-way-to-provide-context-rules-vs-memories/132960)

### Tree-sitter & LSP
- [Does Improving Custom Tree-sitter Extension Grammar Help Cursor Indexer?](https://forum.cursor.com/t/does-improving-custom-tree-sitter-extension-grammar-help-cursor-indexer/81932)
- [Explainer: Tree-sitter vs. LSP](https://news.lavx.hu/article/explainer-tree-sitter-vs-lsp)

### 用户体验
- [Cursor @ Symbol Commands: Complete Context Guide 2026](https://markaicode.com/cursor-at-symbol-commands-explained/)
- [Mastering Context Management in Cursor](https://stevekinney.com/courses/ai-development/cursor-context)
- [Cursor Rules Guide](https://www.johnplummer.com/blog/Cursor+Rules+Guide)

---

## 总结

Cursor 的上下文记忆机制核心是：

1. **自动化优先** — 不依赖用户手动操作
2. **向量检索核心** — 语义搜索比关键词搜索更有效
3. **增量更新** — Merkle Tree + 智能分块
4. **多层次记忆** — 全局/项目/会话/索引
5. **成本优化** — turbopuffer 降低 20x 成本
6. **隐私保护** — 只上传 embedding，不上传代码

remem 应该学习的：
- ✅ 自动捕获（不依赖 Claude 主动调用）
- ✅ 向量检索（投资好的 embedding 模型）
- ✅ 增量更新（避免全量重建）
- ✅ 本地优先（隐私和成本）
- ✅ 零配置（用户体验）

remem 应该避免的：
- ❌ 依赖手动保存
- ❌ 过度设计（知识图谱、多模态）
- ❌ 云端存储（隐私风险）
- ❌ 复杂配置（用户门槛）
