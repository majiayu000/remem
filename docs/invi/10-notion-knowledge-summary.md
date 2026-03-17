# Notion AI 核心设计精华（remem 必读）

> 从 Notion 的 2000 亿 Block 架构中提炼出的关键设计决策

---

## 一、数据模型：Block-Based 统一抽象

### 核心洞察

**一切皆 Block**：文本、图像、列表、数据库行、页面本身——都是 Block。

```rust
struct Block {
    id: Uuid,           // UUID v4
    type: BlockType,    // 定义如何渲染
    properties: Value,  // 存储具体数据
    content: Vec<Uuid>, // 子 Block ID 列表
    parent: Uuid,       // 父 Block ID（权限用）
}
```

### 关键设计决策

**属性与类型解耦**：
- 转换 Block 类型时，属性保持不变，只改变 `type` 字段
- 待办事项 → 标题：`checked` 属性被忽略但保留
- 转换回来时，`checked` 状态仍在

**对 remem 的启示**：
```rust
// Observation 可以有多种"视图"，但底层数据结构统一
struct Observation {
    id: Uuid,
    data: ObservationData,  // 统一存储
    view_type: ViewType,    // narrative | facts | concepts | timeline
}
```

---

## 二、存储架构：分片 + 数据湖

### PostgreSQL 分片

| 维度 | 数值 |
|------|------|
| 逻辑分片 | 480 个 Schema |
| 物理实例 | 96 个 PostgreSQL |
| Sharding Key | Workspace ID |
| 数据规模 | 2000 亿 Block（2024） |

**为什么 480？** 可被 2/3/4/5/6/8/10/12/15/16/20/24/30/32/40/48/60/80/96/120/160/240 整除，支持灵活扩容。

### 数据湖管道

```
Postgres → Debezium CDC → Kafka → Hudi → S3 → Spark
```

**对 remem 的启示**：
- ❌ 不需要 480 分片（remem 是单用户/小团队）
- ❌ 不需要数据湖（remem 不做复杂分析）
- ✅ 需要 CDC 思想：捕获变更而非全量同步

---

## 三、向量搜索：两年三次架构演进

### 演进时间线

| 时间 | 架构 | 成本优化 |
|------|------|----------|
| 2023-11 | 专用 Pod 集群 | 基线 |
| 2024-05 | Serverless | -50% |
| 2025-01 | turbopuffer | -60%（累计 -80%） |

### 增量更新：Page State Project

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

---

## 四、AI 设计：横向层而非垂直功能

### 核心理念

**AI 不是独立功能，而是嵌入到每个工作流的横向层。**

| 用户状态 | AI 工具 |
|----------|---------|
| 新建页面 | 头脑风暴、起草 |
| 编辑内容 | 继续写作、总结 |
| 选中文本 | 改进写作、调整语气 |

### 与传统 Chatbot 的区别

| 维度 | 传统 Chatbot | Notion AI |
|------|--------------|-----------|
| 交互方式 | 空白文本框 | 上下文感知的工具 |
| 理解能力 | 只看用户输入 | 理解页面内容 + 用户意图 |
| 集成方式 | 独立应用 | 嵌入每个 Block |

**对 remem 的启示**：
- ❌ 不要设计 `save_memory` 工具让 Claude "主动保存"
- ✅ 自动捕获对话，在后台提取记忆
- ✅ 在需要时自然呈现记忆（如搜索、上下文注入）

---

## 五、Q&A 与检索

### 检索策略

**双路径**：
- **语义搜索**：向量 embedding + turbopuffer
- **关键词搜索**：PostgreSQL 全文搜索

**跨工具搜索**（AI Connectors）：
- Slack：搜索消息和反馈
- Google Drive：搜索文档
- JIRA/GitHub：搜索项目和代码

**权限控制**：用户只能看到他们在连接工具中已有访问权限的内容。

### 引用机制

- 结果下拉菜单显示 AI 咨询的来源
- "验证页面"用蓝色勾标记
- 用户可深入查看具体文档

**对 remem 的启示**：
```rust
struct SearchResult {
    answer: String,
    sources: Vec<ObservationRef>,  // 引用的 Observation
    confidence: f32,
}

// 搜索时返回来源，让用户验证
```

---

## 六、数据库功能：Relations & Rollups

### Relations（关系）

- 连接不同数据库中的项目
- 双向关系：A 关联 B 时，B 自动出现反向关系

### Rollups（汇总）

- 从关联记录中检索和聚合数据
- 支持 sum/average/count 等聚合函数

**公式示例**：
```javascript
// 汇总关联数据库的数值
prop("RelationProperty").map(current.prop("NumericProperty")).sum()
```

**对 remem 的启示**：
```rust
// Observation 之间的关系
struct ObservationRelation {
    from: ObservationId,
    to: ObservationId,
    relation_type: RelationType,  // references | contradicts | extends
}

// 查询时可以聚合关联的 Observation
fn get_related_facts(obs_id: ObservationId) -> Vec<Fact> {
    relations
        .filter(|r| r.from == obs_id && r.relation_type == RelationType::References)
        .flat_map(|r| get_observation(r.to).facts)
        .collect()
}
```

---

## 七、协作功能：实时同步

### 实时编辑

- 多用户同时编辑，变更实时同步
- 通知在 10 秒内推送

### 评论与提及

- `@` 提及用户，对方在 Inbox 收到通知
- 评论可针对单个 Block 或跨多个 Block

**对 remem 的启示**：
- remem 是单用户系统，不需要实时协作
- 但可以借鉴"评论"机制：用户可以标注 Observation，添加反馈

---

## 八、AI Autofill：智能填充

### 功能

AI Autofill 是数据库属性类型，从页面内容中自动提取信息。

| 类型 | 功能 |
|------|------|
| Summary | 自动总结 |
| Key Info | 提取关键信息 |
| Translation | 翻译 |
| Custom | 自定义提示词 |

**对 remem 的启示**：
```rust
// Observation 的自动提取字段
struct Observation {
    // 用户手动输入
    title: Option<String>,

    // AI 自动提取
    auto_summary: String,        // 自动总结
    auto_keywords: Vec<String>,  // 自动提取关键词
    auto_category: Category,     // 自动分类
}
```

---

## 九、版本控制：Page History

### 功能

- 记录页面的所有编辑、添加、删除
- 可恢复到任意历史版本

**对 remem 的启示**：
```rust
// Observation 的版本历史
struct ObservationVersion {
    observation_id: ObservationId,
    version: u32,
    data: ObservationData,
    created_at: DateTime<Utc>,
    created_by: UserId,
}

// 支持回滚到历史版本
fn restore_version(obs_id: ObservationId, version: u32) -> Result<()> {
    let historical = get_version(obs_id, version)?;
    update_observation(obs_id, historical.data)?;
    Ok(())
}
```

---

## 十、对 remem 的核心启示

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
   - remem 是单用户/小团队，单个 SQLite 足够

2. **数据湖架构**
   - remem 不需要复杂分析，直接在主数据库上建索引

3. **跨工具搜索**
   - 专注做好对话记忆，不要过早扩展

### 🎯 核心原则

1. **质量优先于成本**
   - 不要为了省 API 成本砍掉 LLM 提取能力

2. **自动化优先于手动**
   - 不要依赖 Claude 主动调用 save_memory

3. **增量优先于全量**
   - 不要每次都重新处理整个对话历史

4. **结构化优先于非结构化**
   - Observation 应该有清晰的 schema

---

## 十一、技术栈对比

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

## 十二、实现路线图

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

## 参考资料

完整调研报告：`docs/invi/10-notion-knowledge.md`

核心来源：
- [The data model behind Notion's flexibility](https://www.notion.com/blog/data-model-behind-notion)
- [Two years of vector search at Notion](https://www.notion.com/blog/two-years-of-vector-search-at-notion)
- [Lessons learned from sharding Postgres at Notion](https://www.notion.com/blog/sharding-postgres-at-notion)
- [Building and scaling Notion's data lake](https://www.notion.com/blog/building-and-scaling-notions-data-lake)
- [The design thinking behind Notion AI](https://www.notion.com/blog/the-design-thinking-behind-notion-ai)
