# Augment Code 记忆机制深度调研

## 1. 如何提取（Extraction）

### 1.1 自动上下文收集

**Context Engine 的核心架构**

Augment Code 的记忆系统建立在 **Context Engine** 之上，这是一个专门为代码设计的语义搜索系统。与传统的关键词搜索（"grep with better UX"）不同，Context Engine 通过以下方式自动收集上下文：

- **实时语义索引** — 维护代码库的实时索引，不仅跟踪文件内容，还跟踪代码片段之间的关系
- **依赖图分析** — 解析代码库构建依赖图，理解模块间的调用关系
- **提交历史分析** — 分析 git 提交历史，理解代码变更的 *为什么*，而不仅仅是 *是什么*
- **语义嵌入** — 使用自定义训练的嵌入模型捕获代码的语义关系，而非仅仅语法匹配

**数据管道架构**

Context Connectors 通过六步流程处理内容：

1. **发现（Discover）** — 扫描所有文件，遵循 `.gitignore` 和 `.augmentignore` 规则
2. **过滤（Filter）** — 排除二进制文件和大型文件
3. **哈希计算（Hash）** — 计算文件哈希值检测变化
4. **差异对比（Diff）** — 与存储状态对比，识别新增/修改/删除的文件
5. **嵌入处理（Embed）** — 只有变化的文件被发送到 Context Engine 进行嵌入
6. **状态保存（Save）** — 保存新状态供下次增量运行使用

**增量更新机制**

- 未改变的文件被跳过
- 修改的文件重新索引
- 删除的文件从索引中移除
- 新文件被添加
- 处理速度：**每秒数千文件**

### 1.2 代码库的语义索引

**自定义嵌入模型**

Augment 不使用通用的嵌入 API（如 OpenAI），而是构建了 **自定义 AI 模型**：

> "通用嵌入模型擅长识别哪些文本片段相似。然而，通用模型的嵌入很容易错过大量在文本层面不相似但语义相关的上下文。"

**索引规模**

- 支持 **400,000+ 文件** 的代码库
- 在 SWE-bench 上达到 **70.6% 准确率**（相比文件数量受限的竞品 56%）

**索引速度**

- 初次索引：通常不到 1 分钟（取决于代码库大小）
- 增量更新：文件变更后 **数秒内** 完成索引

### 1.3 用户交互的记忆捕获

**持久化线程（Persistent Threads）**

Augment Code 支持跨会话的持久化对话线程，保留：

- 实现决策
- 团队最佳实践
- 架构选择的推理过程

**模式识别**

Context Engine 不仅查找孤立的匹配，还提取团队约定：

> "当你要求它'为支付请求添加日志'时，它会映射整个请求路径——从 React 组件通过 API 端点到数据库事务——然后分析现有的日志模式以保持一致性。"

**多源上下文集成**

除了代码，Context Engine 还摄取：

- **提交历史** — 理解变更的 *为什么*
- **外部文档** — 通过 GitHub/Linear/Jira/Notion/Confluence 集成
- **MCP 协议** — 支持连接监控工具和自定义 API

### 1.4 跨会话的上下文恢复

**"无限上下文窗口"**

Context Engine 通过智能检索创造"完美知识"的感知：

- 使用语义搜索（非关键词匹配）判断相关性
- 按与请求的接近度排序结果
- 压缩信息同时保留关键模式
- 遵守访问权限

**Remote Agents（2026 年 3 月更新）**

支持完全自主的后台任务执行，保持上下文跨越长时间运行的任务。

---

## 2. 提取什么（What to Extract）

### 2.1 代码结构理解

**AST 级别的分析**

虽然文档未明确提及 AST，但从功能推断 Context Engine 理解：

- 函数签名和调用关系
- 类继承和接口实现
- 模块依赖和导入关系
- 数据库模型和 API 端点的映射

**跨层依赖追踪**

当添加数据库字段时，Augment Code 能够识别需要协调更新的所有层：

- 后端模型定义
- API 验证逻辑
- TypeScript 接口
- 前端表单组件
- 相关测试

### 2.2 用户意图推断

**语义理解而非关键词匹配**

传统搜索需要精确关键词，Augment Code 的语义搜索理解意图：

- 查询 "速率限制实现" 会找到相关代码，即使该短语从未出现
- 理解自然语言规范并转换为代码实现

**Spec-Driven 工作流**

Context Engine 处理自然语言规范，通过三阶段方法：

1. 理解规范的架构意图
2. 在 400k+ 文件的代码库中定位相关代码
3. 生成保持架构一致性的实现

### 2.3 项目知识

**架构决策捕获**

虽然 Augment Code 本身不直接生成 ADR（Architecture Decision Records），但它通过以下方式捕获架构知识：

- 分析提交历史理解决策演变
- 识别代码模式和团队约定
- 保留跨会话的实现决策

**技术栈和约定**

Context Engine 自动学习：

- 项目使用的框架和库
- 代码风格和命名约定
- 错误处理模式
- 测试策略

### 2.4 协作记忆

**团队成员的偏好**

Augment Code 为每个开发者维护 **独立的个性化索引**，而非共享索引：

> "这处理了开发者同时在不同分支上工作的现实——一个函数可能在一个分支上存在，但在另一个分支上不存在。"

**异步协作支持**

通过 AI 中介的异步交接：

- 开发者以 AI 可处理的方式记录思考
- 下一个团队成员上线时，可以查询 AI 关于架构选择
- 接收基于实际观察行为的答案，而非重建的猜测

**上下文丢失问题**

研究显示 **73% 的上下文在交接时丢失**，即使有文档。Augment Code 通过持久化观察代码演变来解决这个问题，作为"跨交接持续存在的机构记忆"。

---

## 3. 如何保存（Storage）

### 3.1 本地 vs 云端存储

**混合方案**

Augment Code 采用第三种路径——混合方案：

- 代码在 **受控环境中处理**，供应商可以访问但承诺不永久存储
- 使用 **Google Cloud 基础设施**：
  - **PubSub** — 消息队列
  - **BigTable** — 分布式存储
  - **AI Hypercomputer** — GPU 加速处理

**本地部署选项**

通过 MCP 集成，Augment 提供两种部署模式：

1. **本地服务器** — 运行 Auggie CLI 作为 MCP 服务器（stdio），实时索引工作目录
2. **远程服务器** — 通过 HTTP 连接到 Augment 托管的 Context Engine

### 3.2 代码索引的数据结构

**向量嵌入 + 元数据**

虽然具体实现未公开，但从功能推断：

- **向量数据库** — 存储代码片段的语义嵌入
- **依赖图** — 存储代码间的关系
- **元数据索引** — 文件路径、行号、提交历史、作者信息

**分层存储**

为了管理 RAM 成本，Augment：

- **共享重叠索引部分** — 同一组织的用户之间共享公共代码的索引
- **Proof of Possession** — 通过加密哈希验证用户只能访问其有权查看的代码

### 3.3 用户偏好的持久化

**个性化索引**

每个开发者拥有独立的索引，支持：

- 不同的分支状态
- 个人的工作区配置
- 自定义的 `.augmentignore` 规则

**记忆与规则系统**

Augment Code 的"记忆与规则"系统在开发会话间保持：

- 实现决策
- 团队最佳实践
- 个人偏好设置

### 3.4 隐私保护

**安全架构**

Augment Code 通过三层机制保护代码隐私：

1. **Proof-of-Possession API** — 代码补全仅在本地拥有的代码上操作，消除复杂的授权管理，防止未授权访问

2. **Non-extractable Architecture** — 防止数据泄露，消除跨租户泄漏，通过不可提取的 API 设计强制执行严格的访问控制

3. **数据隔离与最小化** — 仅收集和处理必要的数据，从不在客户专有数据上训练模型

**合规认证**

- **SOC 2 Type II** — 2024 年 7 月 10 日获得认证
- **ISO/IEC 42001** — AI 管理系统认证

**关键权衡**

文档指出："大多数工具依赖合同禁止而非技术不可能性——它们承诺不在客户代码上训练，但缺乏使未授权学习不可能的架构障碍。"

Augment Code 的 Context Engine 处理 400,000+ 文件，创造了"生产力与隐私风险之间的不舒适权衡"——更大的上下文窗口意味着更全面的模式学习。

---

## 4. 如何更新（Update）

### 4.1 实时索引 vs 批量索引

**实时增量索引**

Augment Code 采用 **实时索引** 策略：

- 文件变更后 **数秒内** 更新索引
- 使用 **PubSub 队列** 平衡实时更新与批量工作负载（新用户入职、模型重新部署）

**批量处理场景**

批量索引用于：

- 新用户首次连接仓库
- 模型升级后重新嵌入
- 大规模重构后的全量重建

### 4.2 代码变更的增量更新

**分支切换处理**

当开发者切换分支、搜索替换或应用格式化变更影响数百个文件时：

- 索引系统通过自定义推理栈处理 **每秒数千文件**
- 维护每个开发者的独立索引，支持不同分支状态

**文件变更检测**

通过哈希计算检测变更：

1. 计算当前文件哈希
2. 与存储状态对比
3. 只重新索引变化的文件
4. 删除已移除文件的索引

### 4.3 记忆的版本控制

**Git 集成**

虽然文档未明确说明记忆的版本控制机制，但从功能推断：

- Context Engine 分析提交历史，理解代码演变
- 支持跨分支的上下文切换
- 保留历史决策的推理过程

**分支感知**

每个开发者的索引是分支感知的：

- 切换分支时，索引自动更新以反映当前分支状态
- 支持同时在多个分支上工作的场景

### 4.4 过期记忆的清理策略

**文档未明确说明**

搜索结果中未找到 Augment Code 关于过期记忆清理、垃圾回收或索引维护的具体信息。

**推测机制**

基于系统设计推测可能的策略：

- **基于访问时间** — 长期未访问的索引可能被降级或归档
- **基于分支状态** — 已合并或删除的分支的索引可能被清理
- **基于存储配额** — 达到存储限制时清理最旧或最少使用的索引

---

## 5. 关键技术洞察

### 5.1 为什么自定义嵌入模型？

**通用模型的局限**

> "通用嵌入模型擅长识别哪些文本片段相似。然而，通用模型的嵌入很容易错过大量在文本层面不相似但语义相关的上下文。"

**自定义模型的优势**

- 理解代码特有的语义关系（如函数调用、数据流）
- 捕获项目特定的模式和约定
- 提高检索的相关性和准确性

### 5.2 个性化索引 vs 共享索引

**为什么选择个性化？**

- 支持多分支并行开发
- 避免不同开发者的上下文污染
- 提供更精确的个人化建议

**如何平衡成本？**

- 共享重叠的索引部分（同一组织的公共代码）
- 使用 Proof of Possession 确保安全性

### 5.3 实时索引的挑战

**性能要求**

- 每秒处理数千文件
- 数秒内完成增量更新
- 支持 400,000+ 文件的代码库

**技术栈**

- Google Cloud PubSub — 消息队列
- BigTable — 分布式存储
- AI Hypercomputer — GPU 加速推理

### 5.4 上下文窗口 vs 智能检索

**"无限上下文窗口"的实现**

Augment Code 不是真正的无限上下文窗口，而是通过智能检索创造这种感知：

1. 语义搜索找到相关代码
2. 按相关性排序
3. 压缩信息保留关键模式
4. 动态注入到 LLM 上下文

**优势**

- 避免上下文窗口限制
- 降低推理成本
- 提高响应速度

---

## 6. 与 remem 的对比

### 6.1 Augment Code 的优势

1. **企业级规模** — 支持 400,000+ 文件，remem 目前未测试此规模
2. **自定义嵌入模型** — 针对代码优化，remem 使用通用 LLM 提取
3. **实时索引** — 数秒内更新，remem 目前是会话结束时批量处理
4. **多源集成** — 支持 GitHub/Jira/Notion 等，remem 目前仅支持代码
5. **团队协作** — 个性化索引 + 共享知识，remem 目前是单用户

### 6.2 remem 的优势

1. **本地优先** — 完全本地运行，无隐私风险，Augment Code 是云端处理
2. **开源透明** — 用户可审计和修改，Augment Code 是闭源商业产品
3. **零成本** — 无订阅费用，Augment Code 需要付费
4. **简单架构** — 易于理解和维护，Augment Code 依赖复杂的云基础设施

### 6.3 remem 应该学习什么？

**必须学习**

1. **自动捕获机制** — 不能依赖 Claude 主动调用 save_memory，需要自动提取
2. **增量更新** — 实时或近实时的索引更新，而非会话结束时批量
3. **语义搜索** — 使用嵌入模型而非简单的文本匹配
4. **分支感知** — 支持多分支并行开发的场景

**可以学习**

1. **自定义嵌入模型** — 如果通用 LLM 提取质量不够，考虑训练专用模型
2. **多源集成** — 支持外部文档、issue tracker 等
3. **团队协作** — 共享知识库 + 个人化索引

**不应该学习**

1. **云端处理** — 保持本地优先的设计哲学
2. **复杂基础设施** — 避免依赖 PubSub/BigTable 等企业级组件
3. **商业化模式** — 保持开源和零成本

---

## 7. 实现建议

### 7.1 短期（1-2 周）

1. **修复自动捕获** — 恢复 LLM 提取能力，不依赖 save_memory 工具
2. **增量索引** — 文件变更时触发重新索引，而非等到会话结束
3. **语义搜索** — 使用嵌入模型（如 text-embedding-3-small）而非纯文本匹配

### 7.2 中期（1-2 月）

1. **分支感知** — 为不同分支维护独立的记忆索引
2. **依赖图分析** — 解析代码构建依赖关系，提高检索相关性
3. **提交历史分析** — 从 git log 中提取架构决策和变更原因

### 7.3 长期（3-6 月）

1. **自定义嵌入模型** — 如果通用模型不够好，训练代码专用的嵌入模型
2. **多源集成** — 支持 GitHub Issues、PR 描述、项目文档等
3. **团队协作** — 支持共享知识库，同时保持个人化索引

---

## 参考资料

### 官方文档

- [Context Connectors: How It Works](https://docs.augmentcode.com/context-services/context-connectors/how-it-works)
- [A real-time index for your codebase](https://www.augmentcode.com/blog/a-real-time-index-for-your-codebase-secure-personal-scalable)
- [Context Engine MCP](https://docs.augmentcode.com/context-services/mcp/overview)
- [Security & Privacy](https://augmentcode.com/security)
- [Workspace Indexing](https://docs.augmentcode.com/setup-augment/workspace-indexing)

### 技术分析

- [How Augment Code Solved the Large Codebase Problem](https://blog.codacy.com/ai-giants-how-augment-code-solved-the-large-codebase-problem)
- [Building a context engine for real codebases](https://www.insprd.io/work/augment)
- [Privacy Comparison of Cloud AI Coding Assistants](https://www.augmentcode.com/guides/privacy-comparison-of-cloud-ai-coding-assistants)

### 应用场景

- [How AI Solves Context Loss for Remote Development Teams](https://www.augmentcode.com/guides/how-ai-solves-context-loss-for-remote-development-teams)
- [10 AI Tactics That Actually End Context Switching](https://www.augmentcode.com/guides/10-ai-tactics-that-actually-end-context-switching-for-full-stack-engineers)
- [13 Best AI Coding Tools for Complex Codebases](https://www.augmentcode.com/guides/13-best-ai-coding-tools-for-complex-codebases)

### 竞品对比

- [Augment Code vs Cursor](https://www.augmentcode.com/tools/augment-code-vs-cursor)
- [Augment Code vs Gemini CLI](https://www.augmentcode.com/tools/augment-code-vs-gemini-cli)
- [Cursor vs. Copilot vs. Augment](https://www.augmentcode.com/guides/cursor-vs-copilot-vs-augment)

### 学术研究

- [Code Context Memory Systems](https://www.emergentmind.com/topics/code-context-memory)
- [Memory-Augmented Language Agents](https://www.emergentmind.com/topics/memory-augmented-language-agents)
- [AST-Guided Adaptive Memory for Repository-Level Code Generation](https://arxiv.org/html/2601.02868v1)

---

## 8. 性能指标与局限性

### 8.1 SWE-bench 性能

**Auggie CLI 的表现**

- **SWE-bench Pro**: 51.80% 成功率（排名第一）
- **SWE-bench Verified**: 65.4% 成功率（开源 agent 第一）
- **整体准确率**: 70.6%（相比文件数量受限的竞品 56%）

**技术实现**

开源 agent 通过结合 Claude 3.7 和 O1 达到 65.4% 成功率，关键技术包括：

- 多模型协作策略
- 代码库全量理解
- 智能测试生成

### 8.2 定价模型

**当前定价（2026 年 3 月）**

Augment Code 采用 **基于积分的定价模型**（2025 年 10 月 20 日起）：

- **Indie 计划**: $20/月，40,000 积分
- **Standard 计划**: $60/月，130,000 积分（最多 20 用户）
- **Max 计划**: $200/月，450,000 积分（最多 20 用户）
- **Enterprise**: 超过 20 用户需联系销售

**积分消耗**

- 平均每次查询：40-70 积分
- Context Engine MCP 查询：40-70 积分

**定价变更历史**

Augment Code 经历了多次定价调整，引发用户抗议：

1. **早期**: 基于消息数量定价（Indie $20/月 125 条消息）
2. **2025 年 5 月**: 改为"更简单的定价"，基于成功处理的消息数
3. **2025 年 10 月**: 改为积分制，用户抱怨实际涨价

### 8.3 已知问题与局限性

**可靠性问题**

根据用户反馈（Medium 文章），Augment Code 存在以下问题：

- **频繁崩溃** — 任务执行中途突然终止，需要重启
- **不稳定性** — 工具优先宣传而非可靠性
- **上下文丢失** — 尽管声称持久化记忆，仍有上下文丢失情况

**规模限制**

- 虽然支持 400,000+ 文件，但实际性能取决于代码库复杂度
- 大规模索引的初始化时间可能较长

**隐私权衡**

- 云端处理意味着代码必须上传到 Augment 服务器
- 尽管有 SOC 2 Type II 认证，但仍存在"生产力与隐私风险之间的不舒适权衡"
- 依赖合同承诺而非技术不可能性来防止训练数据泄露

**成本问题**

- 积分制定价模型使成本难以预测
- 重度使用者可能快速耗尽积分
- 相比本地工具（如 remem），持续订阅成本较高

### 8.4 适用场景

**最适合**

- 企业级大型代码库（100,000+ 文件）
- 需要跨仓库理解的微服务架构
- 远程团队需要异步协作
- 愿意为生产力付费的团队

**不适合**

- 隐私敏感的项目（金融、医疗、国防）
- 小型个人项目（成本不划算）
- 需要完全离线工作的场景
- 预算有限的开源项目

---

## 9. 关键启示

### 9.1 记忆系统的核心原则

通过调研 Augment Code，我们总结出记忆系统的核心原则：

1. **自动化优先** — 不能依赖用户主动保存，必须自动捕获
2. **实时更新** — 增量索引而非批量处理，保持记忆新鲜
3. **语义理解** — 使用嵌入模型理解代码含义，而非仅文本匹配
4. **上下文感知** — 理解代码间的关系，而非孤立的片段
5. **持久化存储** — 跨会话保持记忆，支持长期项目

### 9.2 技术架构的权衡

**云端 vs 本地**

| 维度 | 云端（Augment） | 本地（remem） |
|------|----------------|--------------|
| 性能 | 高（GPU 加速） | 中（依赖本地硬件） |
| 隐私 | 低（需上传代码） | 高（完全本地） |
| 成本 | 高（订阅费用） | 低（零成本） |
| 规模 | 大（400k+ 文件） | 中（待测试） |
| 延迟 | 中（网络往返） | 低（本地处理） |

**自定义模型 vs 通用 LLM**

| 维度 | 自定义嵌入模型 | 通用 LLM 提取 |
|------|---------------|--------------|
| 准确性 | 高（代码优化） | 中（通用能力） |
| 成本 | 高（训练 + 推理） | 低（API 调用） |
| 维护 | 难（需持续训练） | 易（供应商维护） |
| 灵活性 | 低（固定模型） | 高（可切换模型） |

### 9.3 remem 的差异化策略

基于 Augment Code 的调研，remem 应该：

**保持的优势**

1. **本地优先** — 这是核心差异化，不要妥协
2. **零成本** — 开源 + 本地运行，无订阅费用
3. **隐私保护** — 代码永不离开用户机器
4. **简单架构** — 易于理解、修改、贡献

**需要改进的**

1. **自动捕获** — 立即修复，这是致命缺陷
2. **增量索引** — 实时更新而非批量处理
3. **语义搜索** — 使用嵌入模型提高检索质量
4. **分支感知** — 支持多分支并行开发

**可选的增强**

1. **自定义嵌入** — 如果通用 LLM 不够好，考虑训练专用模型
2. **多源集成** — 支持 GitHub Issues、PR、文档等
3. **团队协作** — 共享知识库（可选功能）

### 9.4 实现路线图

**Phase 1: 修复基础（1-2 周）**

- [ ] 恢复自动 LLM 提取（不依赖 save_memory）
- [ ] 实现文件变更监听（fswatch/watchdog）
- [ ] 增量索引触发机制
- [ ] 基础语义搜索（使用 text-embedding-3-small）

**Phase 2: 增强质量（1-2 月）**

- [ ] 分支感知的记忆索引
- [ ] 依赖图分析（基于 AST）
- [ ] 提交历史分析（git log 解析）
- [ ] 记忆去重和合并

**Phase 3: 高级功能（3-6 月）**

- [ ] 评估自定义嵌入模型的必要性
- [ ] 多源集成（GitHub API、本地文档）
- [ ] 团队协作支持（可选）
- [ ] 性能优化（大规模代码库测试）

---

## 10. 总结

### 10.1 Augment Code 的核心价值

Augment Code 通过以下技术实现了企业级代码记忆系统：

1. **Context Engine** — 专为代码设计的语义搜索引擎
2. **自定义嵌入模型** — 理解代码特有的语义关系
3. **实时增量索引** — 数秒内更新，支持 400k+ 文件
4. **个性化索引** — 每个开发者独立索引，支持多分支
5. **多源集成** — 代码 + 提交历史 + 外部文档

### 10.2 remem 的机会

remem 可以通过以下方式差异化：

1. **本地优先** — 完全隐私保护，零成本
2. **简单架构** — 易于理解和贡献
3. **开源透明** — 用户可审计和修改
4. **Claude Code 原生** — 深度集成 Claude Code 工作流

### 10.3 下一步行动

1. **立即修复** — 恢复自动捕获机制（最高优先级）
2. **验证假设** — 测试通用 LLM 提取的质量是否足够
3. **增量迭代** — 先做好基础，再考虑高级功能
4. **用户反馈** — 尽早发布，收集真实使用场景的反馈

**记住核心目标**：remem 的目标是做**最强**的 Claude Code 记忆系统，不是最便宜的。但"最强"不等于"最复杂"——通过本地优先、简单架构、开源透明，remem 可以在隐私、成本、可控性上超越 Augment Code，同时在记忆质量上达到相当水平。

---

## 11. 竞品对比矩阵

### 11.1 上下文能力对比

| 工具 | 文件数量限制 | 上下文窗口 | 跨仓库支持 | 持久化记忆 |
|------|------------|-----------|-----------|-----------|
| **Augment Code** | 400,000+ | 智能检索（无限感知） | ✅ 支持 | ✅ 跨会话 |
| **Cursor** | 50,000 | 272k tokens | ❌ 单仓库 | ⚠️ 会话内 |
| **GitHub Copilot** | 无明确限制 | 64k-128k tokens | ❌ 单仓库 | ❌ 无 |
| **Claude Code** | 取决于上下文窗口 | 200k tokens | ❌ 单仓库 | ⚠️ 通过 MCP |
| **remem** | 待测试 | 智能检索 | ✅ 可支持 | ✅ 本地持久化 |

### 11.2 记忆机制对比

| 维度 | Augment Code | Cursor | GitHub Copilot | remem |
|------|-------------|--------|---------------|-------|
| **自动捕获** | ✅ 实时索引 | ⚠️ 手动索引 | ❌ 无 | ⚠️ 需修复 |
| **语义搜索** | ✅ 自定义嵌入 | ✅ 通用嵌入 | ❌ 关键词 | ⚠️ 计划中 |
| **增量更新** | ✅ 数秒内 | ⚠️ 手动触发 | N/A | ⚠️ 计划中 |
| **分支感知** | ✅ 个性化索引 | ❌ 无 | ❌ 无 | ⚠️ 计划中 |
| **团队协作** | ✅ 共享知识库 | ❌ 无 | ❌ 无 | ⚠️ 可选 |

### 11.3 隐私与成本对比

| 维度 | Augment Code | Cursor | GitHub Copilot | remem |
|------|-------------|--------|---------------|-------|
| **数据存储** | 云端（Google Cloud） | 云端 | 云端（Azure） | 本地 |
| **隐私保护** | SOC 2 Type II | 加密传输 | 企业级加密 | 完全本地 |
| **训练数据** | 承诺不训练 | 承诺不训练 | 承诺不训练 | N/A（本地） |
| **月费用** | $20-200+ | $20-40 | $10-19 | $0 |
| **成本模型** | 积分制（难预测） | 固定订阅 | 固定订阅 | 零成本 |

### 11.4 技术架构对比

| 维度 | Augment Code | Cursor | remem |
|------|-------------|--------|-------|
| **索引技术** | 自定义嵌入模型 | 通用嵌入 | LLM 提取 + 嵌入 |
| **存储后端** | BigTable + PubSub | 未公开 | SQLite |
| **更新机制** | 实时增量 | 手动/定时 | 会话结束（需改进） |
| **部署模式** | 云端 SaaS | 云端 SaaS | 本地 MCP |
| **开源程度** | 部分开源（agent） | 闭源 | 完全开源 |

---

## 12. 开源资源

### 12.1 Augment Code 的开源项目

**augment-agent**

- 仓库：[augmentcode/augment-agent](https://github.com/augmentcode/augment-agent)
- 描述：将 Auggie 集成到开发生命周期的简单包装器
- 用途：研究其 agent 架构和工作流编排

**SWE-bench 开源实现**

- 博客：[#1 open-source agent on SWE-Bench Verified](https://www.augmentcode.com/blog/1-open-source-agent-on-swe-bench-verified-by-combining-claude-3-7-and-o1)
- 成果：65.4% 成功率，结合 Claude 3.7 和 O1
- 价值：学习如何构建高性能代码 agent

### 12.2 相关开源项目

**代码记忆系统**

- [memctl](https://memctl.com/) — 为 AI 编码 agent 提供共享的、分支感知的记忆
- [Beads](https://www.stork.ai/blog/claude-finally-has-a-permanent-brain) — 为 Claude 提供永久的、版本控制的记忆
- [Grov](https://www.productcool.com/product/grov) — 团队间共享和同步的 AI 记忆 + 推理

**代码搜索与索引**

- [Glean](https://glean.software/blog/incremental/) — 增量索引技术
- [Cursor 语义搜索](https://www.zenml.io/llmops-database/enhancing-ai-coding-agent-performance-with-custom-semantic-search) — 自定义嵌入模型训练

---

## 13. 关键引用

### 13.1 Augment Code 的核心洞察

> "通用嵌入模型擅长识别哪些文本片段相似。然而，通用模型的嵌入很容易错过大量在文本层面不相似但语义相关的上下文。"
>
> — [A real-time index for your codebase](https://www.augmentcode.com/blog/a-real-time-index-for-your-codebase-secure-personal-scalable)

> "73% 的上下文在交接时丢失，即使有文档。"
>
> — [How AI Solves Context Loss for Remote Development Teams](https://www.augmentcode.com/guides/how-ai-solves-context-loss-for-remote-development-teams)

> "大多数工具依赖合同禁止而非技术不可能性——它们承诺不在客户代码上训练，但缺乏使未授权学习不可能的架构障碍。"
>
> — [Privacy Comparison of Cloud AI Coding Assistants](https://www.augmentcode.com/guides/privacy-comparison-of-cloud-ai-coding-assistants)

### 13.2 remem 的教训

> "不能依赖 Claude 的'自觉性'来保存记忆。自动化捕获是主力，手动保存只是补充。"
>
> — remem CLAUDE.md

> "调研竞品是为了取各家之长，不是证明可以砍掉 LLM。"
>
> — remem CLAUDE.md

---

## 14. 附录：调研方法

### 14.1 信息来源

1. **官方文档** — Augment Code 的产品文档和技术博客
2. **技术分析** — 第三方评测和竞品对比文章
3. **用户反馈** — Medium、GitHub Issues、Product Hunt 评论
4. **学术研究** — 代码记忆系统的前沿研究论文

### 14.2 调研限制

1. **闭源系统** — Augment Code 的核心实现未公开，部分推断基于功能描述
2. **营销内容** — 官方博客可能夸大优势，需结合用户反馈验证
3. **时效性** — 2026 年 3 月的信息，产品可能持续演进
4. **访问限制** — 部分技术细节（如嵌入模型架构）未公开

### 14.3 验证建议

remem 团队应该：

1. **实际试用** — 注册 Augment Code 免费计划，亲自体验其记忆机制
2. **代码审计** — 研究 augment-agent 开源代码，理解其架构
3. **用户访谈** — 联系 Augment Code 用户，了解真实使用体验
4. **基准测试** — 在相同代码库上对比 Augment Code 和 remem 的记忆质量

---

**调研完成时间**: 2026-03-16
**文档版本**: 1.0
**调研者**: Claude (Opus 4.6)
**字数统计**: 约 8,500 字
