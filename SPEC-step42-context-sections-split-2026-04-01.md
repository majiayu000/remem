# Step 42 - Split context sections module

## Why

`src/context/sections.rs` 当前把 core memory、memory index、workstreams、recent sessions、empty state 五段渲染逻辑和它们的内部 helper 都堆在一个文件里。文件本身已经接近上限，而且这些 renderer 的关注点不同，继续堆在一起会让后续修改上下文输出时不好定位影响面。

本步只做结构拆分，不改变 `generate_context()` 的外部行为，也不改变这些 section 的输出格式。

## Scope

- 保持内部可见接口不变：
  - `render_core_memory()`
  - `render_memory_index()`
  - `render_workstreams()`
  - `render_recent_sessions()`
  - `render_empty_state()`
- 将 `src/context/sections.rs` 拆为 `core`、`index`、`sessions`、`workstreams`、`empty` 子模块
- 保持 Core section 的评分逻辑与字符预算不变
- 保持 Index section 的 display order 不变
- 保持 completed line 120 字符截断规则不变
- 新增一条 core memory 排序回归测试

## Module layout

- `src/context/sections.rs`
  - 模块声明与 `pub use`
- `src/context/sections/core.rs`
  - `render_core_memory` 与评分 helper
- `src/context/sections/index.rs`
  - `render_memory_index` 与 index line helper
- `src/context/sections/sessions.rs`
  - `render_recent_sessions` 与 completed truncation helper
- `src/context/sections/workstreams.rs`
  - `render_workstreams`
- `src/context/sections/empty.rs`
  - `render_empty_state`

## Public interface invariants

- `render_core_memory()` 继续按 score 排序，并遵守 3000 字符 / 6 条上限
- `render_memory_index()` 继续优先输出 decision / bugfix / architecture / discovery / preference / session_activity
- `render_recent_sessions()` 继续只取 `completed` 的第一条非空行，并按 120 字符截断
- `render_workstreams()` 和 `render_empty_state()` 保持现有输出格式

## Validation

定向测试：
- `cargo test render_recent_sessions_truncates_completed_line -- --nocapture`
- `cargo test render_memory_index_prioritizes_known_types -- --nocapture`
- `cargo test render_workstreams_includes_next_action_when_present -- --nocapture`
- `cargo test render_core_memory_prioritizes_higher_score_memories -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不改 context 总装配顺序，也不改 section 标题文案。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
