## Goal

拆分 `src/search.rs`，把 memory search 与 observation search 的流程从同一个文件中分离出来，同时保持 `search::search`、`search::search_with_branch`、`search::search_observations` 的对外调用方式不变。

## Current State

- `src/search.rs` 当前同时包含：
  - FTS query sanitize helper
  - RRF 融合 helper
  - memory 分页 helper
  - memory search 主流程
  - observation search 主流程
- 文件只有 3 个公开入口，但同时承载两条不同数据流：
  - `Memory` 检索：query expand / FTS / entity / temporal / LIKE / RRF / load by ids
  - `Observation` 检索：LIKE / FTS / fallback list / project trim / offset slicing

## Split Plan

- `src/search.rs`
  - 只保留模块组织与 re-export
- `src/search/common.rs`
  - `sanitize_fts_query`
  - `rrf_fuse`
  - `paginate_memories`
- `src/search/memory.rs`
  - `search`
  - `search_with_branch`
- `src/search/observation.rs`
  - `search_observations`

## Constraints

- 不改公开函数名与返回值
- 不改搜索契约：
  - query search 仍走 expand + FTS/LIKE/entity/temporal + RRF
  - 空 query + 空 project 仍返回空结果
  - branch 过滤、offset、include_stale 的当前行为保持不变
- 不顺手修改 ranking、pagination 或 multi-hop 语义

## Non-Goals

- 不拆 `memory_service.rs`
- 不拆 `timeline.rs`
- 不新增测试场景，只复用现有搜索回归测试

## Verification

- 定向测试：
  - `cargo test search_offset_applies_to_memory_pages -- --nocapture`
  - `cargo test branch_filter_happens_before_pagination_for_query_search -- --nocapture`
  - `cargo test standard_search_does_not_implicitly_expand_multi_hop -- --nocapture`
- 完整验证：
  - `cargo check`
  - `cargo test`
