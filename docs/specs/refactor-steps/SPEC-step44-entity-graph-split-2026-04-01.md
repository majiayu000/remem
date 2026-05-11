# Step 44 - Split entity graph module

## Why

`src/entity/graph.rs` 当前把 entity graph expansion 的全部逻辑都堆在一个文件里：seed entity 查询、过滤 SQL 拼装、参数构造和结果去重都混在一起。文件已经比较大，后续如果继续改 project/branch/status 过滤或者扩展策略，影响面不够清楚。

本步只做结构拆分，不改变 entity graph expansion 的公开接口和既有语义。

## Scope

- 保持公开接口不变：
  - `expand_via_entity_graph()`
  - `expand_via_entity_graph_filtered()`
- 将 `src/entity/graph.rs` 拆为 `seed`、`sql`、`expand` 子模块
- 保持现有行为不变：
  - seed memory 为空时返回空
  - 无 entity seed 时返回空
  - 继续排除 seed ids 与 exclude ids
  - 默认只搜索 active memories
  - `branch` 过滤继续允许 `m.branch IS NULL`
- 新增一条过滤回归测试，锁住 branch/status 语义

## Module layout

- `src/entity/graph.rs`
  - 模块声明与 `pub use`
- `src/entity/graph/seed.rs`
  - seed entity 查询
- `src/entity/graph/sql.rs`
  - SQL 和参数构造 helper
- `src/entity/graph/expand.rs`
  - expansion 执行

## Public interface invariants

- `expand_via_entity_graph()` 继续透传到 filtered 版本，默认不加 memory_type / branch，且 `include_inactive=false`
- `expand_via_entity_graph_filtered()` 继续按 shared entity count 降序返回 memory ids
- `include_inactive=false` 时继续只包含 `m.status = 'active'`
- `branch` 提供时继续匹配 `m.branch = ? OR m.branch IS NULL`

## Validation

定向测试：
- `cargo test expand_via_entity_graph_excludes_seed_and_excluded_ids -- --nocapture`
- `cargo test expand_via_entity_graph_filtered_respects_branch_and_status -- --nocapture`
- `cargo test search_by_entity_fallback_matches_partial_name -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不改 entity extraction、不改 entity link 写入。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
