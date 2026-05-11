# Step 33 - Split install module

## Why

`src/install.rs` 当前同时包含 Claude 配置路径、JSON 读写、hook/MCP 配置构造、remem 配置清理和 install/uninstall 运行逻辑，达到 243 行，已经超过项目单文件 200 行限制。这个模块只有 CLI 调用面，适合继续按职责拆分，让路径、配置变换和实际安装流程分层更清楚。

本步只做结构拆分，不改变 `install()` 和 `uninstall()` 的公开接口，也不改变 hooks/MCP 配置写入、清理和提示输出语义。

## Scope

- 保持 `pub fn install()` 和 `pub fn uninstall()` 对外接口不变
- 将 `src/install.rs` 拆为 `paths`、`json_io`、`config`、`runtime`、`tests` 子模块
- 保持 hooks 写入 `~/.claude/settings.json`、MCP 写入 `~/.claude.json` 的语义不变
- 保持 install/uninstall 对 remem hooks 和 remem MCP 的清理规则不变
- 新增纯 JSON 级别回归测试，锁住 hook/MCP 构造与清理行为

## Module layout

- `src/install.rs`
  - 模块声明与 `pub use`
- `src/install/paths.rs`
  - `settings_path`
  - `claude_json_path`
  - `old_hooks_path`
  - `remem_data_dir`
  - `binary_path`
- `src/install/json_io.rs`
  - `read_json_file`
  - `write_json_file`
- `src/install/config.rs`
  - `build_hooks`
  - `build_mcp_server`
  - `is_remem_hook`
  - `remove_remem_hooks`
  - `remove_remem_mcp`
- `src/install/runtime.rs`
  - `install`
  - `uninstall`
- `src/install/tests.rs`
  - hooks/MCP 构造与清理测试

## Public interface invariants

- `install()` 继续先清理已有 remem hooks/MCP，再写入当前 remem 配置
- `install()` 继续创建数据目录，并输出相同的安装提示信息
- `uninstall()` 继续只移除 remem hooks/MCP，不删除数据目录
- `remove_remem_hooks()` 继续只删除指向 remem 的 hooks entry，不碰其他工具 hooks
- `remove_remem_mcp()` 继续删除 key=`remem` 或 command 指向 remem 的 MCP server

## Validation

定向测试：
- `cargo test build_hooks_contains_expected_commands -- --nocapture`
- `cargo test remove_remem_hooks_preserves_other_hooks -- --nocapture`
- `cargo test remove_remem_mcp_removes_named_and_command_matched_servers -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步测试只在内存 JSON 值上验证配置变换，不写真实用户 HOME 下文件。
- 若 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
