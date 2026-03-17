# Investigation Reports (invi)

> 竞品调研与学术研究报告

---

## 目录

### 记忆系统竞品

| 文档 | 主题 | 核心发现 |
|------|------|----------|
| [03-letta-core-memory.md](03-letta-core-memory.md) | Letta Core Memory | 结构化核心记忆（人物/系统） |
| [05-augment-memory.md](05-augment-memory.md) | Augment Memory | 企业级记忆系统架构 |
| [09-rewind-capture.md](09-rewind-capture.md) | Rewind 捕获机制 | 全量捕获 + 本地处理 |
| [09-rewind-capture-summary.md](09-rewind-capture-summary.md) | Rewind 总结 | 捕获策略精华 |

### 知识管理系统

| 文档 | 主题 | 核心发现 |
|------|------|----------|
| [10-notion-knowledge.md](10-notion-knowledge.md) | Notion AI 完整调研 | Block-based 模型 + 向量搜索架构 |
| [10-notion-knowledge-summary.md](10-notion-knowledge-summary.md) | Notion 设计精华 | 增量更新 + 横向 AI 层 |
| [10-notion-findings.md](10-notion-findings.md) | Notion 关键发现 | 对 remem 的设计建议 |

### 学术研究

| 文档 | 主题 | 核心发现 |
|------|------|----------|
| [08-academic-memory-extraction.md](08-academic-memory-extraction.md) | 学术论文综述 | 记忆提取的理论基础 |

---

## 核心洞察

### 1. 数据模型设计

**Notion 的 Block-Based 模型**：
- 一切皆 Block（文本/图像/列表/数据库行/页面）
- 属性与类型解耦，支持灵活转换
- 对 remem：Observation 应该是统一的数据结构，支持多种视图

### 2. 增量更新机制

**Notion 的 Page State Project**：
- 基于哈希的变更检测（xxHash）
- 只处理变更的 span，减少 70% 计算量
- 对 remem：不要每次都重新提取整个对话，只处理新增的 turn

### 3. 自动捕获 vs 手动保存

**Rewind 的全量捕获**：
- 自动捕获所有屏幕内容和音频
- 本地处理，保护隐私
- 对 remem：不要依赖 Claude 主动调用 save_memory，必须自动捕获

**Notion 的横向 AI 层**：
- AI 不是独立功能，而是嵌入到每个工作流
- 上下文感知，在需要时自然呈现
- 对 remem：记忆提取应该自动发生，在需要时自然呈现

### 4. 向量搜索架构

**Notion 的演进**：
- 2023-11：专用 Pod 集群
- 2024-05：Serverless（-50% 成本）
- 2025-01：turbopuffer（-60% 成本，累计 -80%）
- 对 remem：成本优化是持续过程，但从未牺牲质量

### 5. 双路径索引

**Notion 的策略**：
- 离线：Apache Spark 批处理（历史数据）
- 在线：Kafka 消费者（实时更新，亚分钟级延迟）
- 对 remem：批量导入历史对话 + 实时捕获新对话

---

## 对 remem 的设计建议

### ✅ 必须学习

1. **Block-based 统一模型**
   - Observation 是统一的数据结构，支持多种视图
   - 属性与视图类型解耦

2. **增量更新机制**
   - 基于哈希的变更检测
   - 只处理新增/变更的部分
   - 减少 70% 计算量

3. **双路径索引**
   - 离线批处理（历史对话）
   - 在线实时更新（新对话）

4. **横向 AI 层**
   - 自动捕获，不依赖用户主动保存
   - 在需要时自然呈现记忆

5. **引用机制**
   - 搜索结果显示来源
   - 用户可验证信息

### ❌ 不需要的复杂度

1. **480 逻辑分片**
   - Notion 需要支持数百万 Workspace
   - remem 是单用户/小团队，单个 SQLite 足够

2. **数据湖架构**
   - Notion 需要支持复杂分析和 BI
   - remem 不需要 Spark/Hudi/S3

3. **跨工具搜索**
   - Notion 需要集成 Slack/Google Drive/JIRA
   - remem 专注做好对话记忆

---

## 核心原则

### 1. 质量优先于成本
- 不要为了省 API 成本砍掉 LLM 提取能力
- Notion 的成本优化从未牺牲质量

### 2. 自动化优先于手动
- 不要依赖 Claude 主动调用 save_memory
- 必须自动捕获对话

### 3. 增量优先于全量
- 不要每次都重新处理整个对话历史
- 只处理新增/变更的部分

### 4. 结构化优先于非结构化
- Observation 应该有清晰的 schema
- 不是纯文本，而是结构化数据

---

## 技术栈建议

| 组件 | Notion | remem（建议） |
|------|--------|---------------|
| **主存储** | PostgreSQL（96 实例） | SQLite 或单个 PostgreSQL |
| **向量索引** | turbopuffer | Qdrant 或 pgvector |
| **消息队列** | Kafka | 不需要（直接处理） |
| **数据湖** | Hudi + S3 | 不需要 |
| **处理引擎** | Spark | Rust 原生处理 |
| **Embedding** | 自托管（Ray） | OpenAI API 或本地模型 |
| **变更捕获** | Debezium CDC | 应用层捕获 |

---

## 实现路线图

### Phase 1：基础架构（当前）
- [x] SQLite 存储
- [x] Observation 基础 schema
- [ ] 增量提取机制（基于哈希）

### Phase 2：向量搜索
- [ ] 集成向量数据库（Qdrant/pgvector）
- [ ] 双路径索引（离线 + 在线）
- [ ] 引用机制（搜索结果显示来源）

### Phase 3：自动捕获
- [ ] 对话钩子（自动捕获新 turn）
- [ ] 后台提取（不阻塞用户）
- [ ] 增量更新（只处理变更）

### Phase 4：高级功能
- [ ] Observation 关系（references/contradicts/extends）
- [ ] 版本历史（支持回滚）
- [ ] 自动分类和标签

---

## 下一步行动

1. **修复 Zero-LLM 错误**
   - 恢复 LLM 提取能力（不要为了省成本砍功能）
   - 实现增量提取机制（基于哈希，减少 70% 计算量）

2. **实现自动捕获**
   - 对话钩子：每次 Claude 回复后自动触发提取
   - 后台处理：不阻塞用户交互

3. **集成向量搜索**
   - 选择向量数据库（Qdrant 或 pgvector）
   - 实现双路径索引（离线 + 在线）

4. **添加引用机制**
   - 搜索结果显示来源 Observation
   - 用户可验证信息准确性
