# Notion AI 知识管理与记忆机制深度调研

> 调研日期：2026-03-16
> 目标：理解 Notion AI 如何提取、存储、更新和检索知识，为 remem 项目提供设计参考

---

## 1. 核心架构：Block-Based 数据模型

### 1.1 Block 的设计哲学

Notion 的整个架构围绕 **Block** 构建。每个 Block 是一个独立的数据单元，具有四个核心属性：

| 属性 | 说明 |
|------|------|
| **ID** | UUID v4 随机生成的唯一标识符 |
| **Type** | 定义 Block 如何渲染（文本/图像/列表/数据库行等） |
| **Properties** | 存储 Block 的具体数据（如 `title` 存储文本内容） |
| **Content & Parent** | Content 是子 Block ID 的有序数组；Parent 是父 Block ID（用于权限） |

**关键设计决策**：属性存储与 Block 类型解耦。

- 转换 Block 类型时，属性和内容保持不变，只改变 `type` 字段
- 例如：待办事项 → 标题，"已检查"属性被忽略但保留，转换回来时仍可用
- 这种设计保留用户意图，支持灵活的内容类型转换

### 1.2 层级关系与渲染树

Block 通过 `content` 数组形成树状结构：

```
Page Block
├── Text Block ("标题")
├── List Block
│   ├── Text Block ("项目 1")
│   └── Text Block ("项目 2")
└── Database Block
    ├── Row Block (ID: xxx)
    └── Row Block (ID: yyy)
```

**缩进的结构意义**：与传统文字处理器不同，Notion 中的缩进是结构性的——它操纵 Block 之间的关系，而非仅添加样式。

### 1.3 权限模型

由于 Block 可被多个 `content` 数组引用（如同步块），系统使用"向上指针"（`parent` 属性）而非 `content` 数组来实现权限继承。这避免了歧义，并提高了权限检查的效率。

**来源**：[The data model behind Notion's flexibility](https://www.notion.com/blog/data-model-behind-notion)

---

## 2. 存储架构：从单体到分片

### 2.1 PostgreSQL 分片策略

**规模挑战**：
- 2021 年初：200 亿 Block
- 2024 年：2000 亿 Block（数据量翻 10 倍）
- 压缩后数据量：数百 TB

**分片方案**：
- **Sharding Key**：Workspace ID（每个 Block 属于唯一 Workspace）
- **逻辑分片**：480 个逻辑 Schema
- **物理实例**：96 个 PostgreSQL 实例（每个实例 5 个逻辑 Schema）
- **路由**：应用层代码完成 Workspace UUID → 物理数据库 → 逻辑 Schema 的映射

**为什么选 480？**
480 可被 2/3/4/5/6/8/10/12/15/16/20/24/30/32/40/48/60/80/96/120/160/240 整除，支持灵活扩容而无需倍增基础设施。

### 2.2 零停机迁移方案

**三阶段策略**：

1. **双写（Double-write via audit log）**
   新写入同时传播到单体和分片，通过自定义 catch-up 脚本避免逻辑复制瓶颈

2. **回填（Backfill with version comparison）**
   旧数据迁移时跳过已有更新的记录，允许最终一致性

3. **暗读验证（Dark reads）**
   查询同时从两个系统获取数据，对比结果后再完全切换

**实际停机时间**：5 分钟（等待 catch-up 脚本完成待处理写入）

**来源**：
- [Lessons learned from sharding Postgres at Notion](https://www.notion.com/blog/sharding-postgres-at-notion)
- [How does Notion handle 200 billion data entities?](https://vutr.substack.com/p/how-does-notion-handle-200-billion)

---

## 3. 向量搜索架构：两年演进史

### 3.1 架构演进时间线

| 时间 | 架构 | 特点 |
|------|------|------|
| **2023-11** | 专用 Pod 集群 | 存储与计算耦合，按 Workspace ID 分片 |
| **2024-05** | Serverless 架构 | 存储与计算解耦，成本降低 50% |
| **2025-01** | turbopuffer | 基于对象存储，每个 namespace 独立索引，成本再降 60% |

### 3.2 Embedding 模型与基础设施

**模型升级**：
- 迁移到 turbopuffer 时升级到"更高性能的 embedding 模型"（具体模型未公开）
- 从外部 API 提供商迁移到自托管（使用 Ray 框架）

**自托管收益**：
- 消除第三方 API 延迟
- 预计降低 90%+ 的 embedding 基础设施成本
- 统一 GPU/CPU 流水线，消除双重计算开销

### 3.3 索引策略：双路径架构

**离线路径（Offline）**：
- Apache Spark 批处理作业
- 文档分块（chunking）
- 批量加载向量到向量数据库

**在线路径（Online）**：
- Kafka 消费者处理实时页面编辑
- 延迟：亚分钟级（sub-minute latency）

### 3.4 增量更新机制：Page State Project

**问题**：每次编辑都重新处理整个页面，浪费计算资源。

**解决方案**：基于哈希的变更检测

1. **哈希追踪**：
   - 为每个 span（文本片段）计算 xxHash 64-bit 哈希
   - 分别追踪文本内容和元数据的哈希

2. **状态缓存**：
   - 使用 DynamoDB 缓存页面的上一次状态

3. **差异处理**：
   - 只重新 embed 变更的 span
   - 仅元数据变更时，通过向量数据库 API 打补丁（不重新 embed）

**效果**：数据处理量减少 70%

### 3.5 成本与性能优化成果

**turbopuffer 迁移**：
- 搜索引擎成本降低 60%
- AWS EMR 计算成本降低 35%
- 查询延迟从 70-100ms 降至 50-70ms

**Ray 迁移（进行中）**：
- 预计 embedding 基础设施成本降低 90%+

**来源**：
- [Two years of vector search at Notion](https://www.notion.com/blog/two-years-of-vector-search-at-notion)
- [Scaling Vector Search Infrastructure for AI-Powered Workspace Search](https://www.zenml.io/llmops-database/scaling-vector-search-infrastructure-for-ai-powered-workspace-search)

---

## 4. 数据湖架构：CDC 到 S3

### 4.1 数据流管道

```
Postgres (96 实例, 480 分片)
    ↓ Debezium CDC (部署在 AWS EKS)
Kafka (每表一个 topic，合并 480 分片)
    ↓ Apache Hudi
S3 (数据湖存储)
    ↓ Apache Spark
下游系统（分析、AI 特征、BI）
```

### 4.2 技术栈

| 组件 | 技术 |
|------|------|
| **变更捕获** | Debezium CDC connectors（每个 Postgres 主机一个） |
| **消息队列** | Kafka（每表一个 topic） |
| **数据湖存储** | Apache Hudi + S3 |
| **处理引擎** | Apache Spark（轻量级用 PySpark，复杂用 Scala） |
| **云基础设施** | AWS（RDS, EKS, S3） |

### 4.3 增量同步机制

**正常操作**：
- 增量摄取并持续应用 Postgres 变更到 S3
- 延迟：小表几分钟，大表几小时

**Bootstrap 过程**（新表）：
1. AWS RDS export-to-S3 创建初始快照
2. Deltastreamer 从快照时间戳开始读取 Kafka 消息
3. 24 小时内完成数据完整性保证

### 4.4 优化策略

**Hudi 配置**（针对 Notion 的更新密集型工作负载，90% 操作是更新）：
- 使用与 Postgres 相同的 480 分片方案分区
- 按 `event_lsn`（最后更新时间）排序，优先处理最近的 Block
- 使用 Bloom Filter 索引高效剪枝文件

**Spark 处理**：
- 处理复杂的反规范化任务（如跨数十亿 Block 的权限树遍历）
- 小分片在内存中处理，大分片使用磁盘重排

**成果**：
- 摄取时间从"超过一天"降至"小表几分钟，大表几小时"
- 每年节省超过 100 万美元
- 支持 Notion AI 特征

**来源**：[Building and scaling Notion's data lake](https://www.notion.com/blog/building-and-scaling-notions-data-lake)

---

## 5. AI 功能设计：横向 AI 层

### 5.1 设计理念

**核心原则**：AI 不是独立的垂直功能，而是与每个 Block 和界面协作的横向层。

**与传统 Chatbot 的区别**：
- 传统 AI：空白文本框上的 API 包装
- Notion AI：理解页面内容和用户意图，在特定工作流中提供精准建议

### 5.2 上下文感知的 AI 工具

系统根据用户所处状态动态调整工具：

| 用户状态 | 提供的 AI 工具 |
|----------|----------------|
| 新建页面 | 头脑风暴、起草 |
| 编辑现有内容 | 继续写作、总结 |
| 选中文本 | 改进写作、调整语气 |

### 5.3 用户交互设计

**预期性和无缝性**：
- 一键操作（如"改进写作"直接替换文本）
- 对比视图（编辑模式下并排显示原文和 AI 版本）
- 可发现性（通过快捷键 `Space` 和菜单自然呈现）

**来源**：[The design thinking behind Notion AI](https://www.notion.com/blog/the-design-thinking-behind-notion-ai)

---

## 6. Q&A 与企业搜索

### 6.1 检索与排序

- 搜索 Workspace 内容，按来源标签组织结果
- "验证页面"（Verified Pages）用蓝色勾标记，表示可靠性
- 优先级排序基于相关性

### 6.2 答案生成

- 创建"易于理解的 AI 概览"（AI Overviews）
- Research 模式：跨多个来源（内部 Workspace、连接工具、Web 数据）综合信息生成报告

### 6.3 引用机制

- 结果下拉菜单显示 AI 咨询的来源
- 用户可验证信息来源并深入查看具体文档

### 6.4 跨工具搜索（AI Connectors）

**支持的外部数据源**：
- **聊天应用**：Slack（搜索消息和反馈）
- **云存储**：Google Drive, OneDrive, SharePoint
- **项目工具**：JIRA, GitHub（拉取项目、代码、对话）

**权限控制**：
- AI Connectors 尊重权限，用户只能看到他们在连接工具中已有访问权限的内容

**来源**：
- [Find answers and generate reports with enterprise search](https://www.notion.com/help/guides/find-answers-and-generate-reports-with-enterprise-search)
- [Use AI connectors to access more of your team's knowledge](https://www.notion.com/help/guides/use-ai-connectors-to-access-more-of-your-teams-knowledge)

---

## 7. 数据库功能：Relations & Rollups

### 7.1 Relations（关系）

- 连接不同数据库中的项目
- 双向关系：在数据库 A 中关联数据库 B 的项目时，B 中自动出现反向关系

### 7.2 Rollups（汇总）

- 从关联记录中检索和聚合数据
- 支持的聚合函数：sum（求和）、average（平均）、count（计数）等
- 使用 `map()` 和 `sum()` 函数处理关系数组

**公式示例**（汇总关联数据库的数值）：
```
prop("RelationProperty").map(current.prop("NumericProperty")).sum()
```

**来源**：[Relations & rollups](https://www.notion.com/help/relations-and-rollups)

---

## 8. 协作功能：实时同步与通知

### 8.1 实时协作

- 多用户同时编辑，变更实时同步
- 高亮文本添加评论，直接在特定 Block 上交互
- 通知在 10 秒内推送（桌面/移动端）

### 8.2 评论与提及

- 使用 `@` 提及用户，对方在 Inbox 收到通知
- 评论可针对单个 Block 或跨多个 Block 的选中文本
- 快捷键：`cmd/ctrl + shift + M` 快速评论

### 8.3 通知机制

- Inbox 集中显示所有更新
- 可按"所有更新"、"所有评论"、"回复和 @提及"筛选
- 支持桌面推送通知

**来源**：
- [Collaborate with people](https://www.notion.com/help/collaborate-with-people)
- [Comments, mentions & reactions](https://www.notion.com/help/comments-mentions-and-reminders)

---

## 9. AI Autofill：数据库智能填充

### 9.1 功能概述

AI Autofill 是数据库属性类型，使用 AI 从页面内容中提取和总结信息。

### 9.2 预设选项

| 类型 | 功能 |
|------|------|
| **Summary** | 自动总结所有数据库条目 |
| **Key Info** | 提取关键信息 |
| **Translation** | 翻译内容 |
| **Custom** | 自定义提示词，请求特定信息 |

### 9.3 实现方式

1. 打开数据库，创建新属性
2. 在"Suggested"部分后找到"AI Autofill"选项
3. 选择预设类型或自定义提示词
4. AI 根据页面内容自动填充属性值

**来源**：
- [Integrate AI into Your Notion Databases with AI Autofills](http://xray.tech/post/notion-ai-autofills)
- [Using Notion's AI Database Properties](https://ajinkyabhat.com/blog/notion-ai-database-properties)

---

## 10. 版本控制：Page History

### 10.1 功能

- 记录页面的所有编辑、添加、删除
- 类似详细日志，追踪每次变更

### 10.2 访问方式

1. 点击页面右上角的 `...` 菜单
2. 选择"Page History"
3. 查看过去版本的时间线

### 10.3 恢复版本

- 选择历史版本
- 点击"Restore"恢复到该版本

**来源**：
- [How to Effectively Utilize Notion Page History for Version Control](https://ones.com/blog/notion-page-history-version-control/)
- [What Is Page History in Notion?](https://spellapp.com/resources/what-is-page-history-in-notion)

---

## 11. 关键技术决策总结

### 11.1 提取（What to Extract）

| 层级 | 提取内容 |
|------|----------|
| **Block 级** | 文本内容、类型、属性、层级关系 |
| **Page 级** | 标题、正文、Block 树、元数据 |
| **Database 级** | 记录、属性、关系、汇总 |
| **Workspace 级** | 用户权限、协作信息、评论、提及 |

### 11.2 存储（How to Store）

| 组件 | 技术选型 |
|------|----------|
| **主存储** | PostgreSQL（96 实例，480 逻辑分片） |
| **数据湖** | Apache Hudi + S3 |
| **向量索引** | turbopuffer（基于对象存储） |
| **消息队列** | Kafka（CDC 变更流） |
| **状态缓存** | DynamoDB（Page State 哈希） |

### 11.3 更新（How to Update）

| 场景 | 策略 |
|------|------|
| **实时编辑** | Kafka 消费者 + 亚分钟级延迟 |
| **向量索引** | 基于哈希的增量更新（70% 数据量减少） |
| **数据湖同步** | Debezium CDC + Hudi 增量写入 |
| **协作冲突** | 实时同步（推测使用 OT 或 CRDT） |

### 11.4 检索（How to Retrieve）

| 方法 | 技术 |
|------|------|
| **语义搜索** | 向量 embedding + turbopuffer |
| **关键词搜索** | PostgreSQL 全文搜索 |
| **跨工具搜索** | AI Connectors（Slack/Google Drive/JIRA） |
| **数据库查询** | Relations + Rollups + 公式 |

---

## 12. 对 remem 的启示

### 12.1 必须学习的设计

1. **Block-based 模型**
   - 一切皆 Block，统一数据模型
   - 属性与类型解耦，支持灵活转换
   - 对 remem：Observation 可以是统一的 Block 类型，支持多种视图（narrative/facts/concepts）

2. **增量更新机制**
   - 基于哈希的变更检测（Page State Project）
   - 只处理变更部分，减少 70% 计算量
   - 对 remem：不要每次都重新提取整个对话，只处理新增的 turn

3. **双路径索引**
   - 离线批处理 + 在线实时更新
   - 对 remem：批量导入历史对话（离线），实时捕获新对话（在线）

4. **横向 AI 层**
   - AI 不是独立功能，而是嵌入到每个工作流
   - 对 remem：记忆提取不应该是独立的"保存"按钮，而是自动嵌入到对话流程中

### 12.2 不需要的复杂度

1. **480 逻辑分片**
   - Notion 需要支持数百万 Workspace，remem 只需支持单用户或小团队
   - 对 remem：单个 SQLite 或简单的 PostgreSQL 足够

2. **数据湖架构**
   - Notion 需要支持复杂的分析和 BI，remem 只需支持记忆检索
   - 对 remem：不需要 Spark/Hudi/S3，直接在主数据库上建索引

3. **跨工具搜索**
   - Notion 需要集成 Slack/Google Drive/JIRA，remem 只需关注 Claude Code 对话
   - 对 remem：专注做好对话记忆，不要过早扩展到其他数据源

### 12.3 核心原则

1. **质量优先于成本**
   - Notion 从专用集群 → Serverless → turbopuffer，每次迁移都是为了降低成本，但从未牺牲质量
   - 对 remem：不要为了省 API 成本砍掉 LLM 提取能力

2. **自动化优先于手动**
   - Notion 的 AI Autofill 自动填充数据库，用户不需要手动输入
   - 对 remem：不要依赖 Claude 主动调用 save_memory，必须自动捕获

3. **增量优先于全量**
   - Notion 的 Page State Project 只处理变更部分
   - 对 remem：不要每次都重新处理整个对话历史

4. **结构化优先于非结构化**
   - Notion 的 Block 模型提供清晰的结构，而非纯文本
   - 对 remem：Observation 应该有清晰的 schema（narrative/facts/concepts/files），而非纯文本

---

## 参考资料

### 官方博客
- [The data model behind Notion's flexibility](https://www.notion.com/blog/data-model-behind-notion)
- [Two years of vector search at Notion](https://www.notion.com/blog/two-years-of-vector-search-at-notion)
- [Lessons learned from sharding Postgres at Notion](https://www.notion.com/blog/sharding-postgres-at-notion)
- [Building and scaling Notion's data lake](https://www.notion.com/blog/building-and-scaling-notions-data-lake)
- [The design thinking behind Notion AI](https://www.notion.com/blog/the-design-thinking-behind-notion-ai)

### 技术分析
- [Scaling Vector Search Infrastructure for AI-Powered Workspace Search](https://www.zenml.io/llmops-database/scaling-vector-search-infrastructure-for-ai-powered-workspace-search)
- [How does Notion handle 200 billion data entities?](https://vutr.substack.com/p/how-does-notion-handle-200-billion)
- [Examining Notion's Backend Architecture](https://labs.relbis.com/blog/2024-04-18_notion_backend)

### 用户文档
- [Find answers and generate reports with enterprise search](https://www.notion.com/help/guides/find-answers-and-generate-reports-with-enterprise-search)
- [Use AI connectors to access more of your team's knowledge](https://www.notion.com/help/guides/use-ai-connectors-to-access-more-of-your-teams-knowledge)
- [Relations & rollups](https://www.notion.com/help/relations-and-rollups)
- [Collaborate with people](https://www.notion.com/help/collaborate-with-people)
