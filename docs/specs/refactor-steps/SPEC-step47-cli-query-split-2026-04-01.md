# Step 47 - Split CLI query actions module

## Why

`src/cli/actions/query.rs` 当前同时承载 `status`、`search`、`show`、`backfill entities` 四条 CLI 路径，还混着输出格式和回填进度逻辑。文件已经接近单文件上限，后续如果继续改 CLI 查询输出或 entity 回填流程，边界会比较模糊。

本步只做结构拆分，不改变 CLI 子命令、参数和输出语义。

## Scope

- 保持公开接口不变：
  - `run_status()`
  - `run_search()`
  - `run_show()`
  - `run_backfill_entities()`
- 将 `src/cli/actions/query.rs` 拆为 `status`、`search`、`show`、`backfill` 子模块
- 保持既有行为不变：
  - `status` 继续读取共享 stats 并打印数据库/今日活动概览
  - `search` 继续调用 memory search，空结果继续打印 `No results found.`
  - `show` 继续打印 memory 详情
  - `backfill entities` 继续遍历 active memories 并按 100 条打印进度
- 新增 2 条纯逻辑回归测试，钉住搜索预览和时间格式化语义

## Module layout

- `src/cli/actions/query.rs`
  - 模块声明与 `pub use`
- `src/cli/actions/query/status.rs`
  - `run_status`
- `src/cli/actions/query/search.rs`
  - `run_search`
  - 搜索输出 helper
- `src/cli/actions/query/show.rs`
  - `run_show`
  - 时间格式 helper
- `src/cli/actions/query/backfill.rs`
  - `run_backfill_entities`
  - 回填进度逻辑
- `src/cli/actions/query/tests.rs`
  - 纯逻辑回归测试

## Public interface invariants

- `run_search()` 继续默认 `include_stale=false`
- `run_search()` 的 preview 继续只取正文第一行，并限制为 80 个字符
- `run_show()` 继续使用 `%Y-%m-%d %H:%M UTC` 格式
- `run_backfill_entities()` 继续只处理 `status = 'active'` 的 memories

## Validation

定向测试：
- `cargo test cli_query_preview_uses_first_line_and_truncates -- --nocapture`
- `cargo test cli_query_format_memory_timestamp_handles_invalid_epoch -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不改 clap 参数定义，不改 CLI 根调度。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
