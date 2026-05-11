# Step 27 - Split observe module

## Why

`src/observe.rs` 当前同时包含 hook 入口、事件落库与 pending 入队、native Claude memory 同步、frontmatter 解析和路径提取测试，达到 287 行，已经超过项目单文件 200 行限制。后续继续调整 observe pipeline 或 native memory sync 时，职责边界不够清晰。

本步只做结构拆分，不改变 `session_init` / `observe` 的对外接口，也不改变 native memory sync 的现有行为。

## Scope

- 保持 `crate::observe::session_init`、`crate::observe::observe`、`crate::observe::short_path` 的公开接口不变
- 将 `src/observe.rs` 拆为入口、native sync、解析和测试子模块
- 保持 event skip、event classify、event insert、pending enqueue 的现有流程不变
- 保持 native memory 文件 frontmatter 解析与 project path 提取语义不变
- 不修改 `src/cli/mod.rs`、`src/adapter_claude/*` 的调用方式

## Module layout

- `src/observe.rs`
  - 模块声明与 `pub use`
- `src/observe/path.rs`
  - `short_path`
  - `extract_project_from_memory_path`
- `src/observe/native.rs`
  - `sync_native_memory`
- `src/observe/parse.rs`
  - `parse_native_memory_frontmatter`
- `src/observe/hook.rs`
  - `session_init`
  - `observe`
- `src/observe/tests.rs`
  - 现有 native memory parser/path 测试

## Public interface invariants

- `session_init()` 继续从 stdin 读 hook 输入、识别 adapter 并 upsert session
- `observe()` 继续做 skip 判定、bash skip、event classify、event insert、pending enqueue
- `observe()` 继续在 `Write/Edit` 且目标为 Claude native memory 路径时尝试 sync native memory
- `parse_native_memory_frontmatter()` 继续把 `feedback/user` 映射到 `preference`，`project/reference` 映射到 `discovery`
- `extract_project_from_memory_path()` 继续从 `/.claude/projects/<slug>/memory/*.md` 恢复 project path

## Validation

定向测试：
- `cargo test parse_frontmatter_full -- --nocapture`
- `cargo test extract_project_short_slug -- --nocapture`
- `cargo test bash_skip_filter_stays_in_observe_module -- --nocapture`
- `cargo test classify_edit_event -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
