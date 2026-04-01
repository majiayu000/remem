# Step 19 - Split entity module

## Why

`src/entity.rs` 目前同时承载实体抽取、实体链接、按实体搜索、实体图扩展和测试，文件达到 442 行，已经超过项目单文件 200 行限制，也让后续修改 multi-hop/entity search 时影响范围过大。

本步只做结构拆分，不改变公开函数签名与现有搜索语义。

## Scope

- 保持 `crate::entity::*` 的现有公开接口不变
- 将 `src/entity.rs` 拆为按职责分离的子模块
- 新增 entity search / graph expansion 的本地回归测试
- 不修改 `search_multihop.rs`、`search/memory.rs`、`memory/store/write.rs` 的调用方式
- 不改变 stop-word、tech term、branch/status/project 过滤语义

## Module layout

- `src/entity.rs`
  - 模块声明与 `pub use`
- `src/entity/extract.rs`
  - `extract_entities`
  - `is_stop_word`
  - 技术词表
- `src/entity/link.rs`
  - `link_entities`
- `src/entity/search.rs`
  - `search_by_entity`
  - `search_by_entity_filtered`
  - filter SQL helper
- `src/entity/graph.rs`
  - `expand_via_entity_graph`
  - `expand_via_entity_graph_filtered`
- `src/entity/tests.rs`
  - 测试入口
- `src/entity/tests/extract.rs`
  - 现有抽取测试
- `src/entity/tests/search.rs`
  - 搜索与图扩展测试
- `src/entity/tests/support.rs`
  - entity 相关最小 schema helper

## Public interface invariants

- `extract_entities(title, content)` 返回顺序和去重/截断规则保持不变
- `link_entities(conn, memory_id, entities)` 继续执行 entity upsert 与 memory/entity 关联
- `search_by_entity(_filtered)` 继续支持 project / memory_type / branch / include_inactive 过滤
- `expand_via_entity_graph(_filtered)` 继续排除 seed 和 exclude 集合，并按 shared_count 排序后 over-fetch 过滤

## Validation

定向测试：
- `cargo test extract_tool_names -- --nocapture`
- `cargo test extract_from_chinese_mixed -- --nocapture`
- `cargo test search_by_entity_fallback_matches_partial_name -- --nocapture`
- `cargo test expand_via_entity_graph_excludes_seed_and_excluded_ids -- --nocapture`
- `cargo test bench_multi_hop_entity_graph_retrieval -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
