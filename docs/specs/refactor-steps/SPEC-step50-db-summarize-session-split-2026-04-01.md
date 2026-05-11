# Step 50 - Split DB summarize session module

## Why

`src/db/summarize/session.rs` 当前把 session summary finalization、sdk session upsert 和测试都放在同一个文件里。文件已经接近上限，后续如果继续改 summarize cooldown 或 session lifecycle，定位成本会偏高。

本步只做结构拆分，不改变 summarize 写入和 sdk session upsert 的公开接口与既有语义。

## Scope

- 保持公开接口不变：
  - `finalize_summarize()`
  - `upsert_session()`
- 将 `src/db/summarize/session.rs` 拆为 `finalize`、`sdk_session`、`tests` 子模块
- 保持既有行为不变：
  - `finalize_summarize()` 继续在事务里先删旧 summary 再插新 summary，并更新 cooldown
  - `upsert_session()` 继续为同一个 `content_session_id` 复用同一个 `memory_session_id`
  - 再次 upsert 同一条 session 时继续递增 `prompt_counter`
- 新增一条 sdk session 回归测试

## Module layout

- `src/db/summarize/session.rs`
  - 模块声明与 `pub use`
- `src/db/summarize/session/finalize.rs`
  - `finalize_summarize`
- `src/db/summarize/session/sdk_session.rs`
  - `upsert_session`
- `src/db/summarize/session/tests.rs`
  - summarize/session 回归测试

## Public interface invariants

- `finalize_summarize()` 继续返回被替换掉的 summary 条数
- `finalize_summarize()` 继续写入 `summarize_cooldown.last_message_hash`
- `upsert_session()` 继续返回最终持久化的 `memory_session_id`
- `upsert_session()` 继续使用 `mem-` 前缀

## Validation

定向测试：
- `cargo test finalize_summarize_replaces_in_single_commit -- --nocapture`
- `cargo test upsert_session_reuses_memory_session_id_and_increments_counter -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不改 summarize schema，不改 cooldown 行为。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
