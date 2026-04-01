# Step 40 - Split search_multihop module

## Why

`src/search_multihop.rs` 当前把多跳搜索的全部步骤堆在一个文件里：首跳搜索、实体发现、二跳扩展、RRF 融合、结果回装和返回类型都混在一起。虽然文件还在 200 行内，但后续只要调整任一环节，理解和验证成本都会偏高。

本步只做结构拆分，不改变 multi-hop 的公开接口和既有行为语义。

## Scope

- 保持公开接口不变：
  - `search_multi_hop()`
  - `MultiHopResult`
- 将 `src/search_multihop.rs` 拆为 `types`、`discover`、`expand`、`merge`、`search`、`tests` 子模块
- 保持现有行为不变：
  - 首跳仍走 `crate::search::search`
  - query 自身的实体仍不重复作为二跳扩展项
  - 二跳仍同时尝试 entity graph 和 FTS mention search
  - RRF 融合仍保持首跳 1.0x、二跳 0.5x、`k=60`
- 补两条纯逻辑回归测试，锁住实体发现和融合排序行为
- 复用现有集成测试验证 multi-hop 召回不回退

## Module layout

- `src/search_multihop.rs`
  - 模块声明与 `pub use`
- `src/search_multihop/types.rs`
  - `MultiHopResult`
- `src/search_multihop/discover.rs`
  - 首跳结果中的实体发现
- `src/search_multihop/expand.rs`
  - 二跳 ID 扩展
- `src/search_multihop/merge.rs`
  - RRF 融合排序
- `src/search_multihop/search.rs`
  - `search_multi_hop` 总控
- `src/search_multihop/tests.rs`
  - discover / merge 单测

## Public interface invariants

- 空首跳结果继续返回 `hops=1` 和空 `entities_discovered`
- 无新实体可扩展时继续只返回首跳结果
- 无二跳结果时继续保留首跳结果并返回已发现实体
- 二跳融合仍保持 overlap 结果获得累积加权分

## Validation

定向测试：
- `cargo test discover_entities_skips_query_entities_and_deduplicates -- --nocapture`
- `cargo test rank_merged_ids_boosts_overlap_and_respects_limit -- --nocapture`
- `cargo test explicit_multi_hop_returns_related_memories -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不修改 `memory_service`、REST、MCP 对 multi-hop 的启用语义。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
