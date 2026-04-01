# Step 48 - Split CLI root module

## Why

`src/cli/mod.rs` 当前同时包含 clap 类型定义、cwd 参数解析和整个 CLI 总调度。虽然还没超出硬上限，但职责已经明显混在一起，后续如果继续加子命令或调整入口分发，理解和测试成本都会变高。

本步只做结构拆分，不改变 CLI 命令名、参数定义和调度语义。

## Scope

- 保持公开接口不变：
  - `cli::run()`
- 将 `src/cli/mod.rs` 拆为 `types`、`cwd`、`dispatch` 子模块
- 保持既有行为不变：
  - clap 解析仍然发生在 `cli::run()`
  - `Context` / `SyncMemory` 继续复用 cwd 解析 helper
  - 各命令仍然路由到原有 action 或模块入口
- 新增 2 条 cwd 解析回归测试

## Module layout

- `src/cli/mod.rs`
  - 模块声明与 `run()` 入口
- `src/cli/types.rs`
  - `Cli`
  - `Commands`
  - `PreferenceAction`
  - `PendingAction`
- `src/cli/cwd.rs`
  - `resolve_cwd_arg`
- `src/cli/dispatch.rs`
  - `run_cli`
- `src/cli/tests.rs`
  - cwd helper 回归测试

## Public interface invariants

- `cli::run()` 继续执行 `Cli::parse()` 再调度
- `Api` 子命令继续默认端口 `5567`
- `Context` 和 `SyncMemory` 继续在 `cwd` 缺省时回落到当前目录
- 其余命令分发行为保持不变

## Validation

定向测试：
- `cargo test cli_resolve_cwd_arg_prefers_explicit_value -- --nocapture`
- `cargo test cli_resolve_cwd_arg_falls_back_to_current_dir -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不改任何子命令参数定义，不改 action 模块实现。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
