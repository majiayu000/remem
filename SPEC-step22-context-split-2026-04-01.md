# Step 22 - Split context module

## Why

`src/context.rs` 当前同时负责上下文数据加载、memory 评分、各 section 渲染、session summary 查询和空状态输出，达到 368 行，已经超过项目单文件 200 行限制。后续继续调整 context 输出或数据来源时，修改范围不够聚焦。

本步只做结构拆分，不改变 `generate_context` 的对外入口、CLI 调用方式或现有输出结构。

## Scope

- 保持 `crate::context::generate_context` 的公开接口不变
- 将 `src/context.rs` 拆为职责分离的子模块
- 保持 memory 加载顺序、branch 排序、summary/workstream 查询和输出 section 顺序不变
- 新增 context renderer 的最小回归测试
- 不修改 `src/cli/mod.rs` 的调用方式

## Module layout

- `src/context.rs`
  - 模块声明与 `pub use`
- `src/context/format.rs`
  - `format_header_datetime`
  - `type_label`
  - `format_epoch_short`
  - `format_epoch_time`
- `src/context/types.rs`
  - `SessionSummaryBrief`
  - `LoadedContext`
- `src/context/query.rs`
  - `load_context_data`
  - `query_recent_summaries`
- `src/context/render.rs`
  - `generate_context`
- `src/context/sections.rs`
  - `calculate_memory_score`
  - `render_core_memory`
  - `render_memory_index`
  - `render_workstreams`
  - `render_recent_sessions`
  - `render_empty_state`
- `src/context/tests.rs`
  - renderer/formatter 回归测试

## Public interface invariants

- `generate_context(cwd, session_id, use_colors)` 继续忽略当前未使用的 `session_id/use_colors`
- context 输出继续按 `Preferences -> Core -> Index -> WorkStreams -> Sessions` 的顺序拼装
- memory 加载继续先搜项目名，再补 recent memories，并保持当前 branch 优先排序
- 无数据或 DB 打不开时，仍输出 empty state

## Validation

定向测试：
- `cargo test render_recent_sessions_truncates_completed_line -- --nocapture`
- `cargo test render_memory_index_prioritizes_known_types -- --nocapture`
- `cargo test empty_project_produces_report -- --nocapture`
- `cargo test status_handler_matches_shared_system_stats -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
