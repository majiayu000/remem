## Goal

继续收债，拆分 `src/db_query.rs` 和 `src/observe_flush.rs`，把查询层与 flush 流程按已有职责平移到子模块中，同时保持对外调用路径与行为不变。

## Current State

- `src/db_query.rs` 同时包含：
  - observation 行映射与 SQL helper
  - 系统统计查询
  - observation 列表/summary 查询
  - FTS/LIKE 搜索
  - timeline 查询
  - 自测
- `src/observe_flush.rs` 同时包含：
  - existing context 组装
  - session events XML 构造
  - flush 持久化事务
  - Task 单刷逻辑
  - Action batch flush / split-retry / lease backoff
  - 自测

## Split Plan

### db_query

- `src/db_query.rs`
  - 只保留模块组织与 re-export
- `src/db_query/shared.rs`
  - `EPOCH_SECS_ONLY`
  - `map_observation_row`
  - `obs_select_cols`
  - `collect_rows`
  - `push_project_filter`
- `src/db_query/stats.rs`
  - `SystemStats`
  - `DailyActivityStats`
  - `ProjectCount`
  - `query_system_stats`
  - `query_daily_activity_stats`
  - `query_top_projects`
  - `src/db_query/stats/tests.rs` 放相关测试
- `src/db_query/queries.rs`
  - `query_observations`
  - `get_observations_by_ids`
  - `count_active_observations`
  - `get_oldest_observations`
- `src/db_query/summaries.rs`
  - `query_summaries`
  - `get_summary_by_session`
- `src/db_query/search.rs`
  - `search_observations_fts`
  - `search_observations_like`
- `src/db_query/timeline.rs`
  - `get_timeline_around`

### observe_flush

- `src/observe_flush.rs`
  - 只保留模块组织与 re-export
- `src/observe_flush/constants.rs`
  - prompt 常量
  - flush batch / lease / backoff 常量
- `src/observe_flush/context.rs`
  - `build_existing_context`
  - `build_session_events_xml`
  - `src/observe_flush/context/tests.rs` 放 existing context 测试
- `src/observe_flush/persist.rs`
  - `persist_flush_batch`
- `src/observe_flush/runtime.rs`
  - `is_ai_timeout_error`
  - `pending_retry_backoff_secs`
- `src/observe_flush/task.rs`
  - `flush_single_task`
- `src/observe_flush/action.rs`
  - Action batch flush
  - split-retry
  - action batch 结果聚合
- `src/observe_flush/batch.rs`
  - `flush_pending` 总体编排

## Constraints

- 不改对外函数名与调用路径：
  - `db::query_system_stats`
  - `db::query_observations`
  - `db::search_observations_fts`
  - `db::get_timeline_around`
  - `observe_flush::flush_pending`
- 不改 SQL 语义，不顺手调整排序、过滤、分页、重试或 dedup 行为
- 单个新文件控制在 200 行左右，避免把超大文件平移成新的超大文件

## Non-Goals

- 不新增 endpoint / MCP tool / CLI 命令
- 不修改 prompt 内容
- 不新增抽象层，不引入连接池或新的运行时机制

## Verification

- 定向测试：
  - `cargo test query_system_stats_and_related_views_share_one_definition -- --nocapture`
  - `cargo test build_existing_context_includes_observations_and_memories -- --nocapture`
- 完整验证：
  - `cargo check`
  - `cargo test`
