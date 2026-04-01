# Step 31 - Split db_job module

## Why

`src/db_job.rs` 当前同时包含 job 去重入队、lease claim、完成/失败/耗尽/回收等多种状态迁移逻辑，达到 244 行，已经超过项目单文件 200 行限制。这个模块和刚收口的 pending/job 链路是同一层，继续拆开后可以把入队、claim 和状态迁移职责分清，后续调整 worker 重试逻辑时也更容易控制回归范围。

本步只做结构拆分，不改变 `enqueue_job()`、`claim_next_job()`、`mark_job_done()`、`mark_job_failed()`、`mark_job_exhausted()`、`release_expired_job_leases()`、`requeue_stuck_jobs()`、`mark_job_failed_or_retry()` 的对外接口和现有 SQL 语义。

## Scope

- 保持 `crate::db_job::*` 当前公开函数和 `pub use crate::db_models::{Job, JobType};` 不变
- 将 `src/db_job.rs` 拆为 `enqueue`、`claim`、`state`、`tests` 子模块
- 保持 inflight 去重入队语义不变
- 保持按 `priority ASC, created_at_epoch ASC, id ASC` claim 的顺序不变
- 保持 `mark_job_failed_or_retry()` 在达到 `max_attempts` 时转 failed，否则回 pending 并写回退避时间的语义不变
- 新增最小 job queue 回归测试

## Module layout

- `src/db_job.rs`
  - 模块声明、`pub use` 汇总
- `src/db_job/enqueue.rs`
  - `enqueue_job`
- `src/db_job/claim.rs`
  - `claim_next_job`
  - 内部行映射 helper
- `src/db_job/state.rs`
  - `mark_job_done`
  - `mark_job_failed`
  - `mark_job_exhausted`
  - `release_expired_job_leases`
  - `requeue_stuck_jobs`
  - `mark_job_failed_or_retry`
- `src/db_job/tests.rs`
  - job queue 回归测试

## Public interface invariants

- `enqueue_job()` 继续在同 `job_type/project/session_id` 且 state 为 `pending|processing` 时返回现有 job id
- `claim_next_job()` 继续只 claim `pending` 且 `next_retry_epoch <= now` 的 job
- `mark_job_done()` 继续只清理当前 `lease_owner` 持有的 job
- `mark_job_failed()` 继续回 pending、递增 `attempt_count` 并写 `next_retry_epoch`
- `mark_job_failed_or_retry()` 继续在 `next_attempt >= max_attempts` 时标记 `failed`，否则回 `pending`
- `requeue_stuck_jobs()` 继续只是 `release_expired_job_leases()` 的包装

## Validation

定向测试：
- `cargo test enqueue_job_dedups_inflight_job -- --nocapture`
- `cargo test claim_next_job_picks_highest_priority_ready_job -- --nocapture`
- `cargo test mark_job_failed_or_retry_requeues_before_max_attempts -- --nocapture`
- `cargo test mark_job_failed_or_retry_marks_failed_when_exhausted -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 测试直接复用 `crate::migrate::MIGRATIONS[0].sql` 建表，避免手写第二份 jobs schema。
- 若 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
