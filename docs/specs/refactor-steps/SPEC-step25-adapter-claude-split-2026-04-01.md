# Step 25 - Split adapter_claude module

## Why

`src/adapter_claude.rs` 当前同时包含 Claude hook 输入解析、tool skip 规则、bash skip 规则、事件分类和测试，达到 328 行，已经超过项目单文件 200 行限制。后续继续调整 hook 兼容或命令过滤规则时，影响范围不够聚焦。

本步只做结构拆分，不改变 `ClaudeCodeAdapter`、`should_skip_bash_command` 或现有事件分类语义。

## Scope

- 保持 `crate::adapter_claude::ClaudeCodeAdapter` 和 `crate::adapter_claude::should_skip_bash_command` 的对外接口不变
- 将 `src/adapter_claude.rs` 拆为按职责分离的子模块
- 保持现有 action/skip tool 集合、bash skip 前缀和 event summary 规则不变
- 迁移现有测试，不修改 `src/adapter.rs` 调用方式

## Module layout

- `src/adapter_claude.rs`
  - 模块声明与 `pub use`
  - `ClaudeCodeAdapter`
- `src/adapter_claude/constants.rs`
  - tool 分类常量
- `src/adapter_claude/hook.rs`
  - `HookInput`
  - hook 解析 helper
- `src/adapter_claude/bash.rs`
  - `should_skip_bash_command`
  - `is_read_only_polling_cmd`
- `src/adapter_claude/classify.rs`
  - `event_summary`
- `src/adapter_claude/tests.rs`
  - 现有回归测试

## Public interface invariants

- `ClaudeCodeAdapter::parse_hook` 继续从 JSON 中抽取 `session_id/cwd/tool_name/tool_input/tool_response`
- `ClaudeCodeAdapter::should_skip` 继续只保留 `ACTION_TOOLS` 中的行为型工具，并跳过 `SKIP_TOOLS`
- `should_skip_bash_command` 继续把只读搜索/轮询命令判定为 skip，但保留会改状态的 bash 命令
- `event_summary` 继续对 `Edit/Write/NotebookEdit/Bash/Grep/Glob/Agent/Task` 生成当前相同的 summary/event_type

## Validation

定向测试：
- `cargo test skip_read_only_polling_commands -- --nocapture`
- `cargo test keep_mutating_commands -- --nocapture`
- `cargo test classify_edit_event -- --nocapture`
- `cargo test bash_skip_filter_stays_in_observe_module -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
