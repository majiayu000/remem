# Step 16 - Split mcp server module

## Why

`src/mcp/server.rs` 目前同时承载 `MemoryServer` 生命周期、所有 MCP tool 实现、server instructions、启动流程和测试，文件达到 565 行，已经超过项目单文件 200 行限制，也增加了后续修改 MCP 工具时的回归风险。

本步只做结构拆分，不改变 MCP tool 名称、参数模型、响应结构或启动入口。

## Scope

- 保持 `crate::mcp::run_mcp_server` 入口不变
- 保持 `MemoryServer` 的 tool 名、参数类型、返回 JSON 结构不变
- 将 `src/mcp/server.rs` 拆为多个子模块
- 保留现有测试并迁移到独立测试模块
- 不修改 `src/mcp/types.rs`
- 不新增工具、不改变 server instructions 的语义

## Module layout

- `src/mcp/server.rs`
  - 模块声明
  - `MemoryServer` 结构体
  - `new()` / `with_conn()`
  - 组合多个 tool router
- `src/mcp/server/search_tools.rs`
  - `search`
- `src/mcp/server/context_tools.rs`
  - `timeline`
  - `get_observations`
- `src/mcp/server/write_tools.rs`
  - `save_memory`
  - `timeline_report`
- `src/mcp/server/workstream_tools.rs`
  - `workstreams`
  - `update_workstream`
- `src/mcp/server/runtime.rs`
  - `ServerHandler` impl
  - `run_mcp_server`
- `src/mcp/server/tests.rs`
  - 现有 MCP server 单元测试

## Implementation notes

- 使用 `rmcp` 支持的多段 `#[tool_router(router = ..., vis = ...)] impl MemoryServer`，在 `new()` 中组合多个 router
- `MemoryServer::with_conn()` 继续作为统一的数据库打开入口
- 现有日志、错误文案和 JSON 序列化结构保持不变
- 启动阶段的 DB ready 检查逻辑保持不变

## Public interface invariants

- `mcp::run_mcp_server()` 的调用方式不变
- 所有 tool 的名称、description、参数 schema 保持不变
- `search` 的 multi-hop 元信息结构保持不变
- `workstreams` / `update_workstream` 的输出结构保持不变
- 测试命名尽量保持不变

## Validation

定向测试：
- `cargo test sanitize_segment_collapses_invalid_chars -- --nocapture`
- `cargo test resolve_relative_path_from_cwd -- --nocapture`
- `cargo test memory_server_new_does_not_open_database_eagerly -- --nocapture`
- `cargo test search_reopens_database_after_file_removal -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
