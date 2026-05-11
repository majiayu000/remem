## Goal

收敛 `status`、REST `/api/v1/status`、`doctor` 的重复统计 SQL，确保它们读取同一套底层统计定义。

## Current Data Flow

- `src/main.rs::run_status` 直接查询 `memories`、`observations`、`session_summaries`、`pending_observations`
- `src/api.rs::handle_status` 直接查询 `memories`、`observations`
- `src/doctor.rs::check_database` 直接查询 `memories`
- `src/doctor.rs::check_pending_queue` 直接查询 `pending_observations`、`jobs`

这些入口现在各自维护 SQL，统计口径分散。

## Changes

1. 在 `src/db_query.rs` 增加共享统计 DTO 和查询函数：
   - `SystemStats`
   - `DailyActivityStats`
   - `ProjectCount`
   - `query_system_stats`
   - `query_daily_activity_stats`
   - `query_top_projects`
2. `src/api.rs::handle_status` 改为只调用 `query_system_stats`
3. `src/main.rs::run_status` 改为调用共享查询函数，再负责打印
4. `src/doctor.rs::check_database` / `check_pending_queue` 改为调用 `query_system_stats`

## Non-Goals

- 不改 REST/MCP/CLI 的对外字段
- 不新增公开统计字段
- 不调整 `doctor` 的文案规则，只替换底层取数来源

## Verification

- 新增共享查询单测，覆盖 active/stale/pending/failed/stuck job 统计
- 新增 API 状态响应测试，确认 REST 返回值来自共享统计
- 新增 doctor 测试，确认 `check_pending_queue` / `check_database` 与共享统计一致
- 完成后执行：
  - 定向测试
  - `cargo check`
  - `cargo test`
