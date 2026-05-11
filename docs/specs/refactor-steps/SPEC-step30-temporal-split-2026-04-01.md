# Step 30 - Split temporal module

## Why

`src/temporal.rs` 当前同时包含时间表达解析、中文数字解析、time-range memory 查询和内联测试，达到 244 行，已经超过项目单文件 200 行限制。这个模块职责其实很清楚，拆开后能让解析和检索语义分层更清楚，也更方便后续继续调整时间检索逻辑。

本步只做结构拆分，不改变 `extract_temporal()`、`search_by_time()`、`search_by_time_filtered()` 的公开接口，也不改变现有时间表达和 SQL 过滤语义。

## Scope

- 保持 `TemporalConstraint`、`extract_temporal()`、`search_by_time()`、`search_by_time_filtered()` 对外接口不变
- 将 `src/temporal.rs` 拆为 `types`、`parse`、`search`、`tests` 子模块
- 保持中英文时间短语和 `N days ago` / `N天前` 解析语义不变
- 保持 `search_by_time_filtered()` 的 `project` / `memory_type` / `branch` / `include_inactive` SQL 语义不变
- 新增一个最小时间检索过滤回归测试

## Module layout

- `src/temporal.rs`
  - 模块声明与 `pub use`
- `src/temporal/types.rs`
  - `TemporalConstraint`
- `src/temporal/parse.rs`
  - `extract_temporal`
  - `parse_n_days_ago`
  - `parse_last_n_days`
  - `cn_digit`
- `src/temporal/search.rs`
  - `search_by_time`
  - `search_by_time_filtered`
- `src/temporal/tests.rs`
  - 解析回归测试
  - 时间搜索过滤回归测试

## Public interface invariants

- `extract_temporal()` 继续在没有时间表达时返回 `None`
- `extract_temporal()` 继续识别 `yesterday/today/last week/last month/this week/recently` 及中文对应表达
- `search_by_time_filtered()` 继续默认过滤掉 inactive memories
- `search_by_time_filtered()` 继续在 branch 过滤时保留 `branch IS NULL` 的 rows
- `search_by_time()` 继续只是 `search_by_time_filtered()` 的简化包装

## Validation

定向测试：
- `cargo test parse_yesterday -- --nocapture`
- `cargo test parse_n_days_ago_cn -- --nocapture`
- `cargo test no_temporal_in_normal_query -- --nocapture`
- `cargo test search_by_time_filtered_respects_filters -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 时间搜索测试使用内存数据库和最小 memories 数据集，只覆盖过滤语义，不扩展业务行为。
- 若 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
