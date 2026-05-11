# Step 29 - Split pending_admin module

## Why

`src/pending_admin.rs` 当前同时包含 failed pending row 数据结构、failed 列表查询、批量重试和批量清理逻辑。虽然文件只有 132 行，但它处在刚刚拆完的 `db_pending` 同一条 pending 维护链路里，继续把 admin 侧职责拆开，可以让 failed queue 的查询与变更边界更清楚，也便于后续继续收口 pending 相关逻辑。

本步只做结构拆分，不改变 CLI 调用方式，也不改变 failed list / retry / purge 的 SQL 语义。

## Scope

- 保持 `crate::pending_admin::*` 当前公开函数和 `FailedPendingRow` 结构不变
- 将 `src/pending_admin.rs` 拆为 `types`、`query`、`mutate` 和 `tests` 子模块
- 保持 `list_failed()` 的 project 过滤、时间倒序和 limit 语义不变
- 保持 `retry_failed()` 的 failed -> pending 重置语义不变
- 保持 `purge_failed()` 的 project + cutoff 删除语义不变
- 新增最小回归测试，锁住 failed admin 维护行为

## Module layout

- `src/pending_admin.rs`
  - 模块声明与 `pub use`
- `src/pending_admin/types.rs`
  - `FailedPendingRow`
  - 行映射 helper
- `src/pending_admin/query.rs`
  - `list_failed`
- `src/pending_admin/mutate.rs`
  - `retry_failed`
  - `purge_failed`
- `src/pending_admin/tests.rs`
  - failed list / retry / purge 回归测试

## Public interface invariants

- `list_failed()` 继续按 `updated_at_epoch DESC` 返回 failed rows
- `list_failed(project, limit)` 继续只返回对应 project 的 failed rows
- `retry_failed()` 继续把 `failed` rows 重置为 `pending`，并清空 retry / lease / last_error 字段
- `purge_failed()` 继续只删除满足 `status='failed'` 且低于 cutoff 的 rows
- 现有 CLI `pending list/retry/purge` 命令不需要改调用方式

## Validation

定向测试：
- `cargo test list_failed_filters_by_project_and_limit -- --nocapture`
- `cargo test retry_failed_resets_rows_for_selected_project -- --nocapture`
- `cargo test purge_failed_respects_cutoff_and_project -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 测试直接复用 `crate::migrate::MIGRATIONS[0].sql` 建表，避免手写第二份 pending schema。
- 若 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
