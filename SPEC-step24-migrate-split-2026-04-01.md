# Step 24 - Split migrate module

## Why

`src/migrate.rs` 当前同时包含 migration 声明、迁移执行、旧 `user_version` 过渡、dry-run、schema clone 和全部测试，达到 319 行，已经超过项目单文件 200 行限制。继续演进 migration 体系时，定位影响面不够清楚。

本步只做结构拆分，不改变 `run_migrations` / `dry_run_pending` 的对外接口，也不改变现有 migration 语义。

## Scope

- 保持 `crate::migrate::run_migrations` 和 `crate::migrate::dry_run_pending` 的公开接口不变
- 保持 `Migration`、`DryRunResult`、`MIGRATIONS` 和 `OLD_BASELINE_VERSION` 的现有语义
- 将 `src/migrate.rs` 拆为声明、跟踪表、迁移执行、旧系统过渡、dry-run 和测试子模块
- 新增 dry-run 的回归测试
- 不修改 `src/db/core.rs`、`src/doctor/schema.rs` 的调用方式

## Module layout

- `src/migrate.rs`
  - 模块声明与 `pub(crate) use`
- `src/migrate/types.rs`
  - `Migration`
  - `DryRunResult`
  - `MIGRATIONS`
  - `OLD_BASELINE_VERSION`
- `src/migrate/state.rs`
  - `ensure_migration_table`
  - `has_migration_table`
  - `applied_versions`
  - `mark_applied`
- `src/migrate/transition.rs`
  - `transition_from_old_system`
- `src/migrate/run.rs`
  - `run_migrations`
- `src/migrate/dry_run.rs`
  - `dry_run_pending`
  - `clone_schema`
- `src/migrate/tests.rs`
  - 现有测试迁移 + dry-run 回归测试

## Public interface invariants

- `run_migrations` 继续先建 `_schema_migrations`，再执行旧系统过渡，再按版本顺序跑 pending migrations
- baseline 仍然对应旧 `user_version=13`
- `dry_run_pending` 继续在内存库上 clone schema 后执行 pending migrations，不直接修改真实 DB
- 旧版本 `< 13` 的库仍然报错并提示先升级到 `remem v0.3.7`

## Validation

定向测试：
- `cargo test full_migration_on_empty_db -- --nocapture`
- `cargo test dry_run_pending_reports_no_pending_for_current_schema -- --nocapture`
- `cargo test dry_run_pending_reports_pending_for_new_db -- --nocapture`
- `cargo test status_handler_matches_shared_system_stats -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
