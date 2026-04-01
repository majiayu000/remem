# Step 45 - Split entity search module

## Why

`src/entity/search.rs` 当前把 query entity 提取、fallback 分词、过滤 SQL 拼装、参数构造和执行都堆在一个文件里。文件已经偏大，后续如果继续改 project/branch/status 过滤或 query fallback 逻辑，理解和回归成本都会偏高。

本步只做结构拆分，不改变 entity search 的公开接口和既有语义。

## Scope

- 保持公开接口不变：
  - `search_by_entity()`
  - `search_by_entity_filtered()`
- 将 `src/entity/search.rs` 拆为 `runner`、`lookup`、`sql` 子模块
- 保持现有行为不变：
  - query 能抽出实体时优先按精确实体名查
  - 抽不出实体时继续退回到按 query words 模糊匹配
  - 默认只查 active memories
  - `branch` 过滤继续允许 `m.branch IS NULL`
- 新增一条 branch/status 过滤回归测试

## Module layout

- `src/entity/search.rs`
  - 模块声明与 `pub use`
- `src/entity/search/runner.rs`
  - `search_by_entity` / `search_by_entity_filtered`
- `src/entity/search/lookup.rs`
  - fallback 分词与 query 执行
- `src/entity/search/sql.rs`
  - 过滤 SQL helper

## Public interface invariants

- `search_by_entity()` 继续调用 filtered 版本，默认不加 `memory_type` / `branch` 且 `include_inactive=false`
- `search_by_entity_filtered()` 继续去重 memory ids，并保持实体优先、fallback 次之
- fallback 继续忽略长度小于 2 的 query word
- `include_inactive=false` 时继续只返回 `m.status = 'active'`

## Validation

定向测试：
- `cargo test search_by_entity_fallback_matches_partial_name -- --nocapture`
- `cargo test search_by_entity_filtered_respects_branch_and_status -- --nocapture`
- `cargo test expand_via_entity_graph_filtered_respects_branch_and_status -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不改 entity extraction、不改 entity graph expansion。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
