## Goal

拆分 `src/doctor.rs`，把 doctor 的检查模型、环境检查、数据库检查、schema 检查、输出汇总和测试拆成独立模块，同时保持 `doctor::run_doctor` 的公开入口和终端输出格式不变。

## Current State

- `src/doctor.rs` 当前同时包含：
  - `Check` / `Status`
  - binary / hooks / mcp 检查
  - database / pending / schema / disk 检查
  - doctor 输出汇总
  - tests

## Split Plan

- `src/doctor.rs`
  - 只保留模块组织与 re-export
- `src/doctor/types.rs`
  - `Check`
  - `Status`
  - `Check::icon`
- `src/doctor/environment.rs`
  - `check_binary`
  - `check_hooks`
  - `check_mcp`
- `src/doctor/database.rs`
  - `check_database`
  - `check_pending_queue`
  - `check_disk_space`
- `src/doctor/schema.rs`
  - `check_schema_migration`
- `src/doctor/report.rs`
  - `run_doctor`
- `src/doctor/tests.rs`
  - doctor 测试

## Constraints

- 不改公开入口：`doctor::run_doctor`
- 不改检查顺序：
  - Binary
  - Schema
  - Database
  - Hooks
  - MCP server
  - Pending queue
  - Disk usage
- 不改终端 summary 文案和 `ok/WARN/FAIL` 标记

## Non-Goals

- 不改 doctor 的判断阈值
- 不改共享 stats 口径
- 不顺手把 doctor 逻辑并进 API/CLI

## Verification

- 定向测试：
  - `cargo test check_database_reports_shared_active_memory_count -- --nocapture`
  - `cargo test check_pending_queue_reports_shared_counts -- --nocapture`
- 完整验证：
  - `cargo check`
  - `cargo test`
