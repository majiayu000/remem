# Step 28 - Split db_pending module

## Why

`src/db_pending.rs` 当前同时包含 pending queue 数据结构、入队、claim/release、retry/fail/delete 和 session 范围查询，达到 261 行，已经超过项目单文件 200 行限制。继续演进 pending flush 或 worker lease 机制时，职责边界不够清楚。

本步只做结构拆分，不改变 `db::*` 通过 re-export 暴露的 pending API，也不改变现有 SQL 语义。

## Scope

- 保持 `crate::db_pending::*` 现有公开函数和 `PendingObservation` 结构不变
- 将 `src/db_pending.rs` 拆为 types、helper、queue、claim 和 query 子模块
- 保持 lease owner / lease expiry / attempt_count / retry SQL 语义不变
- 新增 pending queue 的最小回归测试
- 不修改 `src/db.rs` 对该模块的 re-export 方式

## Module layout

- `src/db_pending.rs`
  - 模块声明与 `pub use`
- `src/db_pending/types.rs`
  - `PendingObservation`
- `src/db_pending/helpers.rs`
  - `clamp_error`
  - `id_placeholders`
  - `append_ids`
- `src/db_pending/queue.rs`
  - `enqueue_pending`
- `src/db_pending/claim.rs`
  - `claim_pending`
  - `release_pending_claims`
  - `retry_pending_claimed`
  - `fail_pending_claimed`
  - `delete_pending_claimed`
- `src/db_pending/query.rs`
  - `get_stale_pending_sessions`
  - `count_pending`
- `src/db_pending/tests.rs`
  - pending queue 回归测试

## Public interface invariants

- `enqueue_pending()` 继续写入 `status='pending'`、`attempt_count=0`
- `claim_pending()` 继续只 claim 当前 session 下满足重试/lease 条件的 pending rows，并把状态改成 `processing`
- `retry_pending_claimed()` 继续清 lease、保留 `pending` 并写 `next_retry_epoch`
- `fail_pending_claimed()` 继续把状态改成 `failed`
- `delete_pending_claimed()` 继续只删除当前 lease owner 持有、且 `processing` 的 rows
- `count_pending()` 继续只统计当前 session 下可处理的 `pending` rows

## Validation

定向测试：
- `cargo test claim_pending_only_returns_requested_session_rows -- --nocapture`
- `cargo test retry_pending_claimed_resets_status_and_sets_next_retry -- --nocapture`
- `cargo test delete_pending_claimed_only_deletes_processing_rows_for_owner -- --nocapture`
- `cargo test status_handler_matches_shared_system_stats -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
