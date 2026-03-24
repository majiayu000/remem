# remem Benchmark Evaluation Framework

## Goal

量化评估 remem 记忆系统的核心能力，覆盖四个维度：

1. **Memory Capture Rate (MCR)** — 有意义事件被捕获为记忆的比例
2. **Memory Precision@K** — 搜索前 K 条结果中相关条目的比例
3. **Context Relevance Score** — context 输出中包含任务相关记忆的比例
4. **Cross-Session Continuity** — 跨会话决策能否被正确检索和复用

## Architecture

```
tests/benchmark.rs          — 端到端评估测试（#[test]）
tests/bench_fixtures.rs     — 测试数据生成器（模拟编程会话）
```

所有测试使用 in-memory SQLite，不依赖外部 AI 服务。

## Evaluation Scenarios

### Scenario 1: Memory Capture Pipeline
模拟 10 个工具调用事件 → 插入 events → promote to memories → 验证 memories 数量和类型分布。

### Scenario 2: Search Recall & Precision
插入 30 条已知记忆（10 相关 + 20 噪声）→ 执行搜索 → 计算 Precision@5 和 Recall@10。

### Scenario 3: Context Injection Quality
插入混合类型记忆（decision/bugfix/discovery/preference）→ 调用 context 评分逻辑 → 验证 Core 区优先展示高权重记忆。

### Scenario 4: Cross-Session Decision Continuity
Session A 存入 decision → Session B 搜索同一 topic → 验证可检索且排名靠前。

### Scenario 5: Cross-Project Global Memory
Project A 存入 global preference → Project B 查询 → 验证 global scope 记忆可见。

### Scenario 6: Time Decay Ranking
插入新旧两组相同相关度记忆 → 搜索 → 验证新记忆排名高于旧记忆。

### Scenario 7: Summary Parse & Promotion
输入 AI 生成的 summary XML → parse_summary → promote_summary_to_memories → 验证 decisions/discoveries/preferences 被正确提取为独立记忆。

## Quantitative Metrics

| Metric | Formula | Target |
|--------|---------|--------|
| MCR | promoted_memories / meaningful_events | >= 0.8 |
| Precision@5 | relevant_in_top5 / 5 | >= 0.6 |
| Recall@10 | relevant_in_top10 / total_relevant | >= 0.8 |
| Context Score | weighted_relevant_in_core / core_items | >= 0.7 |
| Cross-Session Hit | found_previous_decision ? 1 : 0 | = 1.0 |
| Global Visibility | global_mem_visible_in_other_project ? 1 : 0 | = 1.0 |
| Decay Correctness | newer_ranked_higher ? 1 : 0 | = 1.0 |

## Files Affected

- `tests/benchmark.rs` (new)
- `tests/bench_fixtures.rs` (new)
