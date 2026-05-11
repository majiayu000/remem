# Step 37 - Split log module

## Why

`src/log.rs` 现在同时承载日志路径配置、滚动、文件写入、公开日志函数和 `Timer`。虽然文件尚未超过 200 行，但职责已经混在一起，后续如果继续改日志输出或滚动策略，会让行为边界不够清楚。

本步只做结构拆分，不改变公开接口，也不改变日志文件路径、滚动保留份数、`REMEM_DEBUG` 和 `REMEM_STDERR_TO_LOG` 的现有语义。

## Scope

- 保持公开接口不变：
  - `open_log_append()`
  - `debug()`
  - `info()`
  - `warn()`
  - `Timer`
- 将 `src/log.rs` 拆为配置、写入、计时器、测试子模块
- 保持默认日志文件仍写到 `crate::db::data_dir()/remem.log`
- 保持滚动保留份数和默认大小不变
- 为日志配置和滚动补最小回归测试

## Module layout

- `src/log.rs`
  - 模块声明与 `pub use`
- `src/log/config.rs`
  - 路径、默认大小、轮转目标路径
- `src/log/write.rs`
  - 轮转、日志写入、append 打开、`debug/info/warn`
- `src/log/timer.rs`
  - `Timer`
- `src/log/tests.rs`
  - env 配置、append 打开、轮转回归测试

## Public interface invariants

- `REMEM_LOG_MAX_BYTES` 仍只接受正整数覆盖；无效值继续回退默认值
- `open_log_append()` 继续在需要时创建日志目录，并以 append 模式打开同一个日志文件
- `debug()` 继续只在 `REMEM_DEBUG` 存在时写日志
- `Timer::start()` / `Timer::done()` 继续输出 `START` / `DONE` 日志

## Validation

定向测试：
- `cargo test log_max_bytes_uses_positive_env_override -- --nocapture`
- `cargo test open_log_append_creates_log_file_in_data_dir -- --nocapture`
- `cargo test rotate_if_needed_shifts_existing_files -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不改变日志格式，不新增日志级别。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
