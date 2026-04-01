# Step 49 - Split API handlers module

## Why

`src/api/handlers.rs` 当前把 search/get/save/status 四个 HTTP handler 和请求参数到服务层请求的映射逻辑都堆在一个文件里。文件虽然还未超硬上限，但边界已经比较混杂，后续如果继续扩展 API 行为或修搜索参数契约，回归成本会偏高。

本步只做结构拆分，不改变 REST 路由、请求参数、响应结构和状态码语义。

## Scope

- 保持公开接口不变：
  - `handle_search`
  - `handle_get_memory`
  - `handle_save_memory`
  - `handle_status`
- 将 `src/api/handlers.rs` 拆为 `search`、`show`、`save`、`status` 子模块
- 保持既有行为不变：
  - `search` 继续把 query 参数映射到 `memory_service::SearchRequest`
  - `search.limit` 继续默认 20 且上限 100
  - `search.offset` 继续最小为 0
  - `save` 继续返回 `201 Created`
  - `status` 继续返回 `version/memories/observations`
- 新增 2 条 search 参数映射回归测试

## Module layout

- `src/api/handlers.rs`
  - 模块声明与 `pub use`
- `src/api/handlers/search.rs`
  - `handle_search`
  - 参数映射 helper
- `src/api/handlers/show.rs`
  - `handle_get_memory`
- `src/api/handlers/save.rs`
  - `handle_save_memory`
- `src/api/handlers/status.rs`
  - `handle_status`
- `src/api/tests.rs`
  - 参数映射回归测试继续集中在现有 API 测试文件中

## Public interface invariants

- REST `/search` 继续默认 `include_stale=false`
- REST `/search` 继续默认 `multi_hop=false`
- REST `/search` 继续保留 `branch` 和 `type` 参数含义
- `status` 错误继续返回 `status_failed`

## Validation

定向测试：
- `cargo test search_request_from_params_clamps_limit_and_offset -- --nocapture`
- `cargo test search_request_from_params_preserves_filters -- --nocapture`
- `cargo test status_handler_matches_shared_system_stats -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不改 router 定义，不改 API types。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
