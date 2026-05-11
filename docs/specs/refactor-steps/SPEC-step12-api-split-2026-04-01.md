## Goal

拆分 `src/api.rs`，把 REST API 的 DTO、handler、server 启动和测试拆成独立模块，同时保持 `api::build_router` 和 `api::run_api_server` 的公开入口不变。

## Current State

- `src/api.rs` 当前同时包含：
  - 请求/响应 DTO
  - `DbState`
  - error/db helper
  - search/get/save/status 四个 handler
  - router 构建
  - server 启动
  - tests

## Split Plan

- `src/api.rs`
  - 只保留模块组织与 re-export
- `src/api/types.rs`
  - `DbState`
  - `SearchParams`
  - `SearchResponse`
  - `MultiHopInfo`
  - `MemoryItem`
  - `Meta`
  - `ErrorResponse`
  - `ErrorDetail`
  - `SaveMemoryRequest`
  - `SaveMemoryResponse`
  - `ShowParams`
- `src/api/helpers.rs`
  - `memory_to_item`
  - `error_response`
  - `open_request_db`
- `src/api/handlers.rs`
  - `handle_search`
  - `handle_get_memory`
  - `handle_save_memory`
  - `handle_status`
- `src/api/server.rs`
  - `build_router`
  - `run_api_server`
- `src/api/tests.rs`
  - API tests

## Constraints

- 不改公开入口：
  - `api::build_router`
  - `api::run_api_server`
- 不改 endpoint 路径：
  - `/api/v1/search`
  - `/api/v1/memory`
  - `/api/v1/memories`
  - `/api/v1/status`
- 不改默认参数行为与 JSON 结构

## Non-Goals

- 不改 API 路由
- 不改 memory service 语义
- 不顺手扩展 API 字段

## Verification

- 定向测试：
  - `cargo test db_state_is_stateless -- --nocapture`
  - `cargo test status_handler_reopens_database_after_file_removal -- --nocapture`
  - `cargo test status_handler_matches_shared_system_stats -- --nocapture`
- 完整验证：
  - `cargo check`
  - `cargo test`
