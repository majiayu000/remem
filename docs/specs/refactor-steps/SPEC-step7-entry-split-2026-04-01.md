## Goal

拆分 `src/main.rs` 和 `src/mcp.rs`，让入口分发、参数类型、运行时逻辑各归其位，同时保持对外行为不变。

## Current Pain Points

- `src/main.rs` 同时包含 CLI 参数定义、命令分发、命令实现
- `src/mcp.rs` 同时包含参数类型、tool 实现、服务启动、测试
- 两个文件都已经超过 600 行，入口层和业务层混在一起

## Planned Split

### CLI

- `src/main.rs`
  - 只保留二进制入口
  - 调用 `remem::cli::run()`
- `src/cli/mod.rs`
  - CLI 参数定义
  - 命令分发
- `src/cli/actions.rs`
  - `run_status` / `run_search` / `run_show` / `run_preferences` / `run_pending` 等命令实现

### MCP

- `src/mcp/mod.rs`
  - 对外导出 `run_mcp_server`
- `src/mcp/types.rs`
  - MCP 参数/返回 DTO
- `src/mcp/server.rs`
  - `MemoryServer`
  - tool 实现
  - server 启动逻辑
  - 现有 MCP 单测

## Non-Goals

- 不改 tool 名称、REST 路径、CLI 命令名
- 不调整任何检索/保存/统计逻辑
- 不新增新模块职责之外的抽象层

## Verification

- 现有 MCP/CLI 相关单测保持通过
- 全量执行：
  - `cargo check`
  - `cargo test`
  - `cargo build --release`
- 更新 `docs/ARCHITECTURE.md` 中入口层职责和文件体量描述
