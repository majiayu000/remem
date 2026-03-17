# Zep 知识图谱提取机制深度调研

> 调研日期：2026-03-16
> 目标：理解 Zep/Graphiti 的知识图谱提取流程，为 remem 设计最强记忆系统提供参考

---

## 1. 如何提取：Episode → Entity → Community 三层流水线

### 1.1 整体架构

Zep 采用**三层子图架构**，镜像人类记忆的心理学模型：

```
Episode Subgraph (情节记忆)
    ↓ 提取
Semantic Entity Subgraph (语义记忆)
    ↓ 聚合
Community Subgraph (概念层次)
```

### 1.2 核心提取流程（5 阶段 LLM 调用）

**阶段 1：实体提取（Entity Extraction）**

- **输入**：当前消息 + 前 4 条历史消息（`EPISODE_WINDOW_LEN = 4`）
- **Prompt**：`extract_nodes.py`
  - `extract_message`：对话消息（自动提取说话者）
  - `extract_json`：结构化 JSON 数据
  - `extract_text`：纯文本
- **输出**：`ExtractedEntity` 列表（name + entity_type_id）
- **关键逻辑**：
  - 说话者（冒号前的部分）强制提取为第一个实体
  - 代词消解（he/she/they → 实际名称）
  - 支持自定义 ontology（Pydantic BaseModel 定义实体类型）

**阶段 2：实体去重（Entity Deduplication）**

- **混合搜索**：向量相似度（name_embedding）+ BM25 全文搜索
- **候选筛选**：
  1. 精确匹配（normalize 后字符串相等）
  2. MinHash 相似度（Jaccard > 0.8）
  3. 向量余弦相似度（> 0.95）
- **LLM 判断**：`dedupe_nodes.py` 的 `node` prompt
  - 输入：新实体 + 候选实体列表 + 上下文消息
  - 输出：`duplicate_name`（空字符串表示无重复）
- **合并策略**：
  - 保留 UUID 较小的实体作为 canonical
  - 更新 summary（合并新旧信息）
  - 构建 `uuid_map: dict[str, str]` 用于边指针重定向

**阶段 3：关系提取（Edge Extraction）**

- **Prompt**：`extract_edges.py` 的 `edge` prompt
- **输入**：
  - 当前消息
  - 已提取的实体列表（仅 name + labels）
  - 前 4 条历史消息
  - reference_time（用于解析相对时间）
  - 可选：edge_types（自定义关系类型 ontology）
- **输出**：`ExtractedEdges`（三元组列表）
  ```python
  class Edge(BaseModel):
      source_entity_name: str
      target_entity_name: str
      relation_type: str  # SCREAMING_SNAKE_CASE
      fact: str  # 自然语言描述
      valid_at: str | None  # ISO 8601
      invalid_at: str | None
  ```
- **验证**：
  - 实体名称必须在已提取实体列表中（否则丢弃）
  - 关系类型匹配 edge_type_map 的签名（如 `(User, Organization) → WORKS_AT`）

**阶段 4：关系去重（Edge Deduplication）**

- **混合搜索**：向量相似度（fact_embedding）+ BM25
- **LLM 判断**：`dedupe_edges.py` 的 `resolve_edge` prompt
  - 输入：
    - 新边（NEW FACT）
    - 现有边列表（EXISTING FACTS）
    - 失效候选边列表（FACT INVALIDATION CANDIDATES）
  - 输出：
    ```python
    class EdgeDuplicate(BaseModel):
        duplicate_facts: list[int]  # 仅从 EXISTING FACTS 中选
        contradicted_facts: list[int]  # 可从两个列表中选
    ```
- **失效机制**：
  - 如果新边与旧边矛盾（如"Alice 在 Google 工作" vs "Alice 在 Meta 工作"）
  - 设置旧边的 `invalid_at = 新边的 valid_at`
  - 旧边不删除，保留历史

**阶段 5：社区检测（Community Detection）**

- **算法**：Label Propagation（标签传播）
  - 每个节点初始化为独立社区
  - 迭代：节点采用邻居的多数社区标签
  - 收敛：无节点改变社区
- **触发条件**：`update_communities=True` 或调用 `build_communities()`
- **社区节点**：
  - 类型：`CommunityNode`
  - 属性：summary（LLM 生成的社区描述）
  - 边：`HAS_MEMBER` 连接到实体节点

---

## 2. 提取什么：实体、关系、事实的表示

### 2.1 实体类型（Entity Types）

**默认 Ontology**（`ontology/default_ontology.py`）：

| 类型 | 描述 | 优先级 |
|------|------|--------|
| `User` | 用户（单例，通过 user_id 识别） | 最高 |
| `Assistant` | AI 助手（单例） | 最高 |
| `Preference` | 用户偏好（"我喜欢 X"） | 高 |
| `Organization` | 组织、公司 | 中 |
| `Document` | 文档、书籍、视频 | 中 |
| `Event` | 事件、会议 | 中 |
| `Location` | 地点 | 中 |
| `Topic` | 话题、领域 | 低（兜底） |
| `Object` | 物品、设备 | 低（兜底） |

**自定义 Ontology**：

```python
class Project(BaseModel):
    """代表一个软件项目"""
    project_name: str
    repository_url: str | None

entity_types = {
    "Project": Project,
    "User": User,
}
```

### 2.2 关系类型（Edge Types）

**默认关系**：

- 无预定义类型，LLM 自由生成（如 `WORKS_AT`, `LIVES_IN`, `IS_FRIENDS_WITH`）
- 格式：`SCREAMING_SNAKE_CASE`

**自定义关系 + 签名约束**：

```python
class WorksAt(BaseModel):
    """表示雇佣关系"""
    ...

edge_types = {"WORKS_AT": WorksAt}
edge_type_map = {
    ("User", "Organization"): ["WORKS_AT"],
    ("User", "Project"): ["CONTRIBUTES_TO"],
}
```

### 2.3 事实表示（Fact Representation）

**EntityEdge 结构**：

```python
class EntityEdge(Edge):
    name: str  # relation_type
    fact: str  # 自然语言描述（用于向量检索）
    episodes: list[str]  # 来源 episode UUID 列表
    created_at: datetime  # 系统摄入时间（T'）
    expired_at: datetime | None  # 系统失效时间（T'）
    valid_at: datetime  # 事实生效时间（T）
    invalid_at: datetime | None  # 事实失效时间（T）
    fact_embedding: list[float]  # 768 维向量
    attributes: dict[str, Any]  # 自定义属性
```

**示例**：

```json
{
  "uuid": "abc123",
  "source_node_uuid": "user-alice",
  "target_node_uuid": "org-google",
  "name": "WORKS_AT",
  "fact": "Alice works at Google as a software engineer",
  "valid_at": "2024-01-15T00:00:00Z",
  "invalid_at": "2025-03-01T00:00:00Z",
  "created_at": "2024-01-20T10:30:00Z",
  "expired_at": null
}
```

---

## 3. 如何保存：Neo4j Schema + Bitemporal 模型

### 3.1 图数据库 Schema

**节点类型**：

```cypher
// Episode 节点（情节记忆）
(:Episodic {
  uuid: string,
  name: string,
  content: string,  // 原始消息内容
  source: string,  // "message" | "json" | "text"
  source_description: string,
  group_id: string,
  created_at: datetime,
  valid_at: datetime,  // 消息发送时间
  entity_edges: list<string>  // 关联的边 UUID
})

// Entity 节点（语义记忆）
(:Entity {
  uuid: string,
  name: string,
  summary: string,  // LLM 生成的摘要（<250 字符）
  name_embedding: list<float>,  // 768 维
  group_id: string,
  created_at: datetime,
  attributes: map  // 自定义属性（如 email, role_type）
})

// Community 节点（概念层次）
(:Community {
  uuid: string,
  name: string,
  summary: string,  // 社区描述
  group_id: string,
  created_at: datetime
})
```

**边类型**：

```cypher
// 实体关系（语义边）
(:Entity)-[:RELATES_TO {
  uuid: string,
  name: string,  // relation_type
  fact: string,
  fact_embedding: list<float>,
  episodes: list<string>,
  created_at: datetime,
  expired_at: datetime | null,
  valid_at: datetime,
  invalid_at: datetime | null,
  attributes: map
}]->(:Entity)

// Episode 提及实体
(:Episodic)-[:MENTIONS]->(:Entity)

// 社区成员
(:Community)-[:HAS_MEMBER]->(:Entity)

// Episode 链（Saga）
(:Episodic)-[:NEXT_EPISODE]->(:Episodic)
```

### 3.2 Bitemporal 时间模型

**两条时间线**：

1. **T（事件时间线）**：事实在真实世界中的有效期
   - `valid_at`：事实生效时间
   - `invalid_at`：事实失效时间
2. **T'（事务时间线）**：数据在系统中的存在期
   - `created_at`：边被创建的时间
   - `expired_at`：边被删除的时间（软删除）

**时间戳解析**：

- **绝对时间**：直接提取（"2024-01-15"）
- **相对时间**：基于 `reference_time` 计算
  - "last week" → `reference_time - 7 days`
  - "yesterday" → `reference_time - 1 day`
- **模糊时间**：
  - 仅年份 → `YYYY-01-01T00:00:00Z`
  - 仅日期 → `YYYY-MM-DDT00:00:00Z`

**失效检测**：

```python
# dedupe_edges.py 的 resolve_edge prompt
contradicted_facts: list[int]  # LLM 判断哪些旧边被新边推翻

# 更新旧边
old_edge.invalid_at = new_edge.valid_at
old_edge.expired_at = None  # 保留在图中
```

**查询示例**：

```cypher
// 查询当前有效的关系
MATCH (a:Entity)-[r:RELATES_TO]->(b:Entity)
WHERE r.invalid_at IS NULL OR r.invalid_at > datetime()
RETURN a, r, b

// 查询历史关系（2024-06-01 时的状态）
MATCH (a:Entity)-[r:RELATES_TO]->(b:Entity)
WHERE r.valid_at <= datetime('2024-06-01T00:00:00Z')
  AND (r.invalid_at IS NULL OR r.invalid_at > datetime('2024-06-01T00:00:00Z'))
RETURN a, r, b
```

---

## 4. 如何更新：增量更新 + 去重 + 失效

### 4.1 实体更新策略

**去重后的合并**：

```python
# node_operations.py
async def resolve_extracted_nodes(...):
    # 1. 混合搜索找候选
    candidates = await search(...)

    # 2. LLM 判断重复
    duplicate_name = await llm_dedupe(...)

    # 3. 合并 summary
    if duplicate_name:
        existing_node = await EntityNode.get_by_name(...)
        new_summary = await llm_summarize(
            existing_summary + new_context
        )
        existing_node.summary = new_summary
        await existing_node.save(driver)
```

**Summary 更新 Prompt**（`extract_nodes.py`）：

```python
def extract_summary(context):
    return [
        Message(role='system', content='生成实体摘要'),
        Message(role='user', content=f"""
        <MESSAGES>{context['previous_episodes'] + context['episode_content']}</MESSAGES>
        <ENTITY>{context['node']}</ENTITY>

        合并 MESSAGES 中的新信息和 ENTITY 的现有 summary。
        摘要必须 <250 字符。
        """)
    ]
```

### 4.2 边更新策略

**去重逻辑**：

```python
# edge_operations.py
async def resolve_extracted_edge(...):
    # 1. 混合搜索找相似边
    existing_edges = await search(fact_embedding, ...)

    # 2. LLM 判断重复 + 矛盾
    result = await llm_dedupe_edge(
        new_edge, existing_edges, invalidation_candidates
    )

    # 3. 处理重复
    if result.duplicate_facts:
        return existing_edges[result.duplicate_facts[0]]

    # 4. 处理矛盾（失效旧边）
    for idx in result.contradicted_facts:
        old_edge = all_edges[idx]
        old_edge.invalid_at = new_edge.valid_at
        await old_edge.save(driver)

    # 5. 保存新边
    await new_edge.save(driver)
```

**失效候选筛选**：

```python
# 只检查"可能矛盾"的边
invalidation_candidates = [
    edge for edge in existing_edges
    if edge.source_node_uuid == new_edge.source_node_uuid
    and edge.target_node_uuid == new_edge.target_node_uuid
    and edge.name == new_edge.name
]
```

### 4.3 社区更新策略

**重新计算触发条件**：

- 新增实体节点
- 新增实体关系
- 调用 `build_communities(group_ids)`

**更新流程**：

```python
# community_operations.py
async def build_communities(driver, llm_client, embedder, group_ids):
    # 1. 删除旧社区
    await remove_communities(driver, group_ids)

    # 2. Label Propagation 聚类
    clusters = await get_community_clusters(driver, group_ids)

    # 3. 为每个 cluster 生成 summary
    summaries = await semaphore_gather(*[
        summarize_community(llm_client, cluster)
        for cluster in clusters
    ])

    # 4. 创建 CommunityNode + HAS_MEMBER 边
    for cluster, summary in zip(clusters, summaries):
        community = CommunityNode(name=f"Community-{uuid}", summary=summary)
        await community.save(driver)
        edges = build_community_edges(cluster, community)
        await save_edges(driver, edges)
```

---

## 5. 并发优化：13+ Prompt 的并行执行

### 5.1 并发架构

**Semaphore 限流**：

```python
# helpers.py
SEMAPHORE_LIMIT = int(os.getenv('SEMAPHORE_LIMIT', '10'))

async def semaphore_gather(*tasks):
    semaphore = asyncio.Semaphore(SEMAPHORE_LIMIT)
    async def limited_task(task):
        async with semaphore:
            return await task
    return await asyncio.gather(*[limited_task(t) for t in tasks])
```

**批量提取**（`bulk_utils.py`）：

```python
async def extract_nodes_and_edges_bulk(
    clients, episode_tuples, ...
):
    # 并行提取所有 episode 的实体
    extracted_nodes_bulk = await semaphore_gather(*[
        extract_nodes(clients, episode, previous_episodes, ...)
        for episode, previous_episodes in episode_tuples
    ])

    # 并行提取所有 episode 的关系
    extracted_edges_bulk = await semaphore_gather(*[
        extract_edges(clients, episode, nodes, ...)
        for episode, nodes in zip(episodes, extracted_nodes_bulk)
    ])
```

### 5.2 Prompt 分工统计

**单个 Episode 的 LLM 调用**：

| 阶段 | Prompt | 并发数 | 模型 |
|------|--------|--------|------|
| 实体提取 | `extract_nodes.extract_message` | 1 | large |
| 实体去重 | `dedupe_nodes.node` | N（实体数） | small |
| 实体摘要 | `extract_nodes.extract_summary` | M（需更新的实体数） | small |
| 关系提取 | `extract_edges.edge` | 1 | large |
| 关系去重 | `dedupe_edges.resolve_edge` | K（关系数） | small |
| 社区摘要 | `summarize_nodes.summarize_pair` | C（社区数） | small |

**总计**：`2 + N + M + K + C` 次 LLM 调用（大部分并行）

**批量处理**（10 个 Episode）：

```python
# 10 个 episode 并行提取
await semaphore_gather(*[
    extract_nodes(...) for _ in range(10)  # 10 次并行
])

# 每个 episode 的去重也并行
await semaphore_gather(*[
    dedupe_node(...) for node in all_nodes  # N 次并行
])
```

**实际并发数**：受 `SEMAPHORE_LIMIT` 限制（默认 10），避免 LLM API 429 错误。

---

## 6. 关键设计决策

### 6.1 为什么用 LLM 去重而非纯向量？

**问题**：向量相似度无法处理语义等价但表述不同的实体

- "Apple Inc." vs "Apple" vs "苹果公司"
- "NYC" vs "New York City"

**解决**：三层漏斗

1. 向量 + BM25 召回候选（快速筛选）
2. MinHash 精确匹配（deterministic）
3. LLM 最终判断（语义理解）

### 6.2 为什么不删除旧边？

**Bitemporal 模型的优势**：

- 支持历史查询（"2024 年 6 月时 Alice 在哪工作？"）
- 审计追踪（谁在什么时候修改了什么）
- 回滚能力（撤销错误的失效操作）

**代价**：

- 存储开销（保留所有历史边）
- 查询复杂度（需过滤 `invalid_at`）

### 6.3 为什么用 Label Propagation 而非 Louvain？

**Label Propagation 优势**：

- 实现简单（50 行代码）
- 无需调参
- 增量更新友好（局部重新计算）

**Louvain 劣势**：

- 需要全局模块度优化
- 计算复杂度高
- 不适合频繁更新的图

---

## 7. 对 remem 的启示

### 7.1 必须保留的设计

1. **LLM 提取 + 去重**：这是核心，不能砍
   - 纯向量检索无法处理语义等价
   - 去重是记忆质量的关键
2. **Bitemporal 时间模型**：支持历史查询
3. **混合检索**：向量 + BM25 + 图遍历
4. **Episode 作为 provenance**：可追溯到原始数据

### 7.2 可以优化的地方

1. **社区检测**：
   - 初期可以不做（复杂度高，收益不明确）
   - 等实体数 > 1000 再考虑
2. **批量处理**：
   - 单个 episode 处理延迟 ~2-5 秒
   - 批量处理可降低到 ~0.5 秒/episode
3. **Prompt 优化**：
   - Zep 的 prompt 很长（500+ tokens）
   - 可以精简为更短的版本
4. **缓存策略**：
   - LLM 响应缓存（相同输入 → 相同输出）
   - 向量检索结果缓存

### 7.3 remem 的差异化方向

**Zep 的定位**：通用对话记忆（聊天机器人、客服）

**remem 的定位**：开发者工作记忆（Claude Code）

**差异化设计**：

1. **代码感知的实体类型**：
   - `File`, `Function`, `Class`, `Bug`, `Feature`
   - 而非 `User`, `Organization`, `Document`
2. **工作流感知的关系**：
   - `DEPENDS_ON`, `FIXES`, `IMPLEMENTS`, `REFACTORS`
   - 而非 `WORKS_AT`, `LIVES_IN`
3. **时间粒度**：
   - Zep：天级别（对话历史）
   - remem：分钟级别（开发会话）
4. **检索优化**：
   - Zep：语义相似度优先
   - remem：时间局部性 + 文件关联性优先

---

## 8. 实现路线图

### Phase 1：基础提取（MVP）

- [ ] 实体提取（基于 Graphiti 的 prompt）
- [ ] 关系提取
- [ ] 向量 + BM25 混合检索
- [ ] LLM 去重
- [ ] SQLite 存储（简化版 schema）

### Phase 2：时间模型

- [ ] Bitemporal 时间戳
- [ ] 边失效机制
- [ ] 历史查询 API

### Phase 3：优化

- [ ] 批量处理
- [ ] Prompt 缓存
- [ ] 增量更新

### Phase 4：高级特性

- [ ] 社区检测
- [ ] 自定义 ontology
- [ ] 图可视化

---

## 参考资料

- **论文**：[Zep: A Temporal Knowledge Graph Architecture for Agent Memory](https://arxiv.org/abs/2501.13956)
- **代码**：[getzep/graphiti](https://github.com/getzep/graphiti)
- **文档**：[Graphiti Documentation](https://help.getzep.com/graphiti)
- **博客**：
  - [How do you search a Knowledge Graph?](https://blog.getzep.com/how-do-you-search-a-knowledge-graph/)
  - [Beyond Static Knowledge Graphs](https://blog.getzep.com/beyond-static-knowledge-graphs/)
  - [State of the Art in Agent Memory](https://blog.getzep.com/state-of-the-art-agent-memory/)

---

**结论**：Zep/Graphiti 的知识图谱提取机制是目前开源方案中最成熟的。remem 应该**吸收其核心设计**（LLM 提取 + 去重 + Bitemporal 模型），同时**针对开发者场景优化**（代码感知的 ontology + 时间局部性检索）。不要为了省成本砍掉 LLM 提取能力——这是记忆质量的基石。
