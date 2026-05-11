# Step 26 - Split memory_service module

## Why

`src/memory_service.rs` 当前同时包含 search/save 请求与返回类型、本地笔记路径与写盘辅助、memory search 语义和 save_memory 逻辑，达到 267 行，已经超过项目单文件 200 行限制。后续继续调整 API/MCP 搜索语义或本地 copy 行为时，职责边界不够清楚。

本步只做结构拆分，不改变 `search_memories` / `save_memory` 的对外接口或现有行为契约。

## Scope

- 保持 `crate::memory_service::*` 现有公开类型与函数接口不变
- 将 `src/memory_service.rs` 拆为类型、本地 copy helper、搜索和保存子模块
- 保持精确 `has_more`、显式 `multi_hop` 行为和本地 note 默认路径语义不变
- 新增 memory service 的最小回归测试
- 不修改 API/MCP/CLI 对 `memory_service` 的调用方式

## Module layout

- `src/memory_service.rs`
  - 模块声明与 `pub use`
- `src/memory_service/types.rs`
  - `SearchRequest`
  - `MultiHopMeta`
  - `SearchResultSet`
  - `SaveMemoryRequest`
  - `SaveMemoryResult`
- `src/memory_service/local_copy.rs`
  - env 常量
  - `sanitize_segment`
  - `resolve_local_note_path`
  - 本地 note content/write helper
- `src/memory_service/search.rs`
  - `search_memories`
- `src/memory_service/save.rs`
  - `save_memory`
- `src/memory_service/tests.rs`
  - memory service 回归测试

## Public interface invariants

- `search_memories` 继续在标准搜索路径上 over-fetch `limit + 1` 并精确计算 `has_more`
- `search_memories` 继续只有显式 `multi_hop=true` 时才走 multi-hop 路径
- `save_memory` 继续默认 project=`manual`、title=`Memory`、memory_type=`discovery`
- 本地 note 继续默认落到 `data_dir()/manual-notes/<project>/<timestamp>-<slug>.md`
- `sanitize_segment` 继续把非法字符压成单个 `_`，空结果时回退到 fallback

## Validation

定向测试：
- `cargo test memory_service_reports_exact_has_more -- --nocapture`
- `cargo test standard_search_does_not_implicitly_expand_multi_hop -- --nocapture`
- `cargo test explicit_multi_hop_returns_related_memories -- --nocapture`
- `cargo test resolve_local_note_path_makes_relative_paths_absolute -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
