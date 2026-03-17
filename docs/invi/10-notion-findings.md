# Notion AI 调研总结：关键发现与 remem 设计建议

## 调研完成

已完成 Notion AI 知识管理与记忆机制的深度调研，生成两份文档：

1. **完整调研报告**：`docs/invi/10-notion-knowledge.md`（493 行）
   - 涵盖 Notion 的数据模型、存储架构、向量搜索、数据湖、AI 设计、Q&A、协作等 12 个主题
   - 包含技术细节、架构演进、成本优化、性能指标

2. **核心设计精华**：`docs/invi/10-notion-knowledge-summary.md`（387 行）
   - 提炼出对 remem 最有价值的设计决策
   - 明确标注"必须学习"和"不需要的复杂度"
   - 提供具体的代码示例和实现路线图

---

## 核心发现

### 1. Block-Based 统一模型

**Notion 的核心洞察**：一切皆 Block（文本、图像、列表、数据库行、页面）。

**关键设计**：属性与类型解耦
- 转换 Block 类型时，属性保持不变，只改变 `type` 字段
- 待办事项 → 标题：`checked` 属性被忽略但保留，转换回来时仍在

**对 remem 的启示**：
```rust
// Observation 可以有多种"视图"，但底层数据结构统一
struct Observation {
    id: Uuid,
    data: ObservationData,  // 统一存储
    view_type: ViewType,    // narrative | facts | concepts | timeline
}
```

### 2. 增量更新机制（Page State Project）

**问题**：每次编辑都重新处理整个页面，浪费 70% 计算。

**解决方案**：
1. 为每个 span 计算 xxHash（文本 + 元数据分别哈希）
2. DynamoDB 缓存上一次状态
3. 只重新 embed 变更的 span
4. 仅元数据变更时，通过向量 DB API 打补丁

**效果**：数据处理量减少 70%

**对 remem 的启示**：
```rust
// 不要每次都重新提取整个对话
struct ConversationState {
    last_turn_hash: u64,
    processed_turns: Vec<TurnId>,
}

// 只处理新增的 turn
fn extract_incremental(conv: &Conversation, state: &ConversationState) -> Vec<Observation> {
    conv.turns
        .iter()
        .skip(state.processed_turns.len())
        .map(|turn| extract_from_turn(turn))
        .collect()
}
```

### 3. 横向 AI 层（不是垂直功能）

**核心理念**：AI 不是独立功能，而是嵌入到每个工作流的横向层。

| 用户状态 | AI 工具 |
|----------|---------|
| 新建页面 | 头脑风暴、起草 |
| 编辑内容 | 继续写作、总结 |
| 选中文本 | 改进写作、调整语气 |

**与传统 Chatbot 的区别**：
- 传统：空白文本框，只看用户输入
- Notion：上下文感知，理解页面内容 + 用户意图

**对 remem 的启示**：
- ❌ 不要设计 `save_memory` 工具让 Claude "主动保存"
- ✅ 自动捕获对话，在后台提取记忆
- ✅ 在需要时自然呈现记忆（如搜索、上下文注入）

### 4. 向量搜索架构演进

| 时间 | 架构 | 成本优化 |
|------|------|----------|
| 2023-11 | 专用 Pod 集群 | 基线 |
| 2024-05 | Serverless | -50% |
| 2025-01 | turbopuffer | -60%（累计 -80%） |

**关键优化**：
- 从外部 API 迁移到自托管 embedding（Ray 框架）
- 预计降低 90%+ 的 embedding 基础设施成本
- 查询延迟从 70-100ms 降至 50-70ms

**对 remem 的启示**：
- 成本优化是持续过程，但从未牺牲质量
- 自托管 embedding 可以显著降低成本（如果规模足够大）

### 5. 双路径索引

**离线路径**：Apache Spark 批处理作业，文档分块，批量加载向量

**在线路径**：Kafka 消费者处理实时页面编辑，延迟亚分钟级

**对 remem 的启示**：
```rust
// 离线：批量导入历史对话
fn import_historical_conversations(convs: Vec<Conversation>) -> Result<()> {
    for conv in convs {
        let observations = extract_all(conv);
        batch_insert(observations)?;
    }
    Ok(())
}

// 在线：实时捕获新对话
fn on_new_turn(turn: Turn) -> Result<()> {
    let observation = extract_from_turn(turn);
    insert_observation(observation)?;
    Ok(())
}
```

---

## 必须学习的设计

### ✅ 1. Block-based 统一模型
- Observation 是统一的数据结构，支持多种视图
- 属性与视图类型解耦

### ✅ 2. 增量更新机制
- 基于哈希的变更检测
- 只处理新增/变更的部分
- 减少 70% 计算量

### ✅ 3. 双路径索引
- 离线批处理（历史对话）
- 在线实时更新（新对话）

### ✅ 4. 横向 AI 层
- 自动捕获，不依赖用户主动保存
- 在需要时自然呈现记忆

### ✅ 5. 引用机制
- 搜索结果显示来源
- 用户可验证信息

---

## 不需要的复杂度

### ❌ 1. 480 逻辑分片
- Notion 需要支持数百万 Workspace
- remem 是单用户/小团队，单个 SQLite 足够

### ❌ 2. 数据湖架构
- Notion 需要支持复杂分析和 BI
- remem 不需要 Spark/Hudi/S3，直接在主数据库上建索引

### ❌ 3. 跨工具搜索
- Notion 需要集成 Slack/Google Drive/JIRA
- remem 专注做好对话记忆，不要过早扩展

---

## 核心原则

### 1. 质量优先于成本
- Notion 从专用集群 → Serverless → turbopuffer，每次迁移都是为了降低成本
- 但从未牺牲质量（查询延迟反而降低）
- **对 remem**：不要为了省 API 成本砍掉 LLM 提取能力

### 2. 自动化优先于手动
- Notion 的 AI Autofill 自动填充数据库，用户不需要手动输入
- **对 remem**：不要依赖 Claude 主动调用 save_memory，必须自动捕获

### 3. 增量优先于全量
- Notion 的 Page State Project 只处理变更部分
- **对 remem**：不要每次都重新处理整个对话历史

### 4. 结构化优先于非结构化
- Notion 的 Block 模型提供清晰的结构，而非纯文本
- **对 remem**：Observation 应该有清晰的 schema（narrative/facts/concepts/files）

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

---

## 参考资料

### 完整调研报告
- `docs/invi/10-notion-knowledge.md`（493 行）

### 核心设计精华
- `docs/invi/10-notion-knowledge-summary.md`（387 行）

### 官方来源
- [The data model behind Notion's flexibility](https://www.notion.com/blog/data-model-behind-notion)
- [Two years of vector search at Notion](https://www.notion.com/blog/two-years-of-vector-search-at-notion)
- [Lessons learned from sharding Postgres at Notion](https://www.notion.com/blog/sharding-postgres-at-notion)
- [Building and scaling Notion's data lake](https://www.notion.com/blog/building-and-scaling-notions-data-lake)
- [The design thinking behind Notion AI](https://www.notion.com/blog/the-design-thinking-behind-notion-ai)
