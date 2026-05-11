# Step 20 - Split cli actions module

## Why

`src/cli/actions.rs` 当前同时承载 preference、pending、status/search/show、entity backfill、eval、encrypt、cleanup 等多类 CLI 行为，文件达到 385 行，已经超过项目单文件 200 行限制，也让后续继续调整某一类命令时更容易把不相关入口混在一起改动。

本步只做结构拆分，不改变 `remem` CLI 的命令名、参数、输出语义或现有调用路径。

## Scope

- 保持 `cli::mod` 对 actions 的调用方式不变
- 将 `src/cli/actions.rs` 拆为按职责分离的子模块
- 保持 `run_preferences`、`run_pending`、`run_status`、`run_search`、`run_show`、`run_backfill_entities`、`run_eval_local`、`run_eval`、`run_encrypt`、`run_cleanup` 的行为不变
- 不修改 `src/cli/mod.rs` 的命令定义
- 不新增 CLI 命令，不改变已有 stdout 文本格式

## Module layout

- `src/cli/actions.rs`
  - 模块声明与 `pub(super) use`
- `src/cli/actions/shared.rs`
  - `resolve_cwd_project`
- `src/cli/actions/preferences.rs`
  - `run_preferences`
- `src/cli/actions/pending.rs`
  - `run_pending`
- `src/cli/actions/query.rs`
  - `run_status`
  - `run_search`
  - `run_show`
  - `run_backfill_entities`
- `src/cli/actions/eval.rs`
  - `run_eval_local`
  - `run_eval`
  - eval dataset DTO
- `src/cli/actions/maintenance.rs`
  - `run_encrypt`
  - `run_cleanup`

## Public interface invariants

- `cli::mod` 继续通过 `use actions::{...}` 引入所有 `run_*` 入口
- preference / pending / status / search / show / eval / encrypt / cleanup 的命令输出保持现状
- `run_eval(dataset, k)` 继续从 JSON 数据集读取 `queries` 并调用现有评测指标
- `run_backfill_entities()` 继续直接从 active memories 全量回填 entity 关系

## Validation

定向测试：
- `cargo test test_add_and_remove_preference -- --nocapture`
- `cargo test status_handler_matches_shared_system_stats -- --nocapture`
- `cargo test search_offset_applies_to_memory_pages -- --nocapture`
- `cargo test bench_topic_key_dedup -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
