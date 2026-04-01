# Step 46 - Split search memory module

## Why

`src/search/memory.rs` 当前把 memory search 的 query 路径、channel 收集、RRF 融合、结果回装和无 query 的 listing 路径都堆在一个文件里。文件已经偏大，后续如果继续改 query channel 或 queryless list 语义，边界会不够清楚。

本步只做结构拆分，不改变 `search()` / `search_with_branch()` 的公开接口和既有行为。

## Scope

- 保持公开接口不变：
  - `search()`
  - `search_with_branch()`
- 将 `src/search/memory.rs` 拆为 `runner`、`text`、`listing` 子模块
- 保持现有行为不变：
  - query 路径继续使用 FTS/entity/temporal/LIKE 四个 channel
  - channel 为空时继续返回空结果
  - queryless 路径继续要求 `project` 非空，否则返回空
  - queryless + branch 继续委托 `memory::list_memories`
- 新增一条 queryless branch 过滤回归测试

## Module layout

- `src/search/memory.rs`
  - 模块声明与 `pub use`
- `src/search/memory/runner.rs`
  - `search` / `search_with_branch`
- `src/search/memory/text.rs`
  - query 路径的 channel 收集与结果回装
- `src/search/memory/listing.rs`
  - queryless listing 路径

## Public interface invariants

- `search()` 继续透传到 `search_with_branch(..., None)`
- `search_with_branch()` 继续按 `page_target = limit + offset + 1` 的 over-fetch 语义执行
- query 路径继续以 RRF 融合 channel，并保持分页发生在最终有序结果之后
- queryless 路径继续在 `project` 为空时返回空向量

## Validation

定向测试：
- `cargo test search_queryless_with_branch_filters_memories -- --nocapture`
- `cargo test search_include_stale_controls_archived_memories -- --nocapture`
- `cargo test branch_filter_happens_before_pagination_for_query_search -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不改 multi-hop 逻辑，也不改 observation search。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
