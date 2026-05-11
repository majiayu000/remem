# Step 41 - Split memory events module

## Why

`src/memory/events.rs` 当前把 event 写入、event 查询、memory/session 统计、旧 event 清理、stale memory 归档和内联测试都放在一个文件里。虽然文件还没超过硬限制，但职责已经明显混合，后续如果继续改 maintenance 或 session 汇总逻辑，理解和回归成本都会偏高。

本步只做结构拆分，不改变 `crate::memory::*` 对外暴露的事件相关接口和行为语义。

## Scope

- 保持公开接口不变：
  - `insert_event()`
  - `get_session_events()`
  - `get_recent_events()`
  - `cleanup_old_events()`
  - `archive_stale_memories()`
  - `count_session_memories()`
  - `get_session_files_modified()`
  - `count_session_events()`
- 将 `src/memory/events.rs` 拆为 `write`、`query`、`cleanup`、`tests` 子模块
- 保持 `get_session_files_modified()` 继续按首次出现顺序去重返回文件
- 新增一条 files 去重测试

## Module layout

- `src/memory/events.rs`
  - 模块声明与 `pub use`
- `src/memory/events/write.rs`
  - `insert_event`
- `src/memory/events/query.rs`
  - 查询与统计函数
- `src/memory/events/cleanup.rs`
  - old event cleanup / stale memory archive
- `src/memory/events/tests.rs`
  - 现有测试与 files 去重回归

## Public interface invariants

- `insert_event()` 继续按当前时间写入 `created_at_epoch`
- `get_session_events()` 继续按 `created_at_epoch ASC` 返回
- `get_recent_events()` 继续按 `created_at_epoch DESC LIMIT ?` 返回
- `cleanup_old_events()` / `archive_stale_memories()` 继续用 `days * 86400` 计算 cutoff
- `get_session_files_modified()` 继续只统计 `file_edit` / `file_create`，忽略无效 JSON，并去重

## Validation

定向测试：
- `cargo test test_event_insert_and_query -- --nocapture`
- `cargo test test_get_session_files_modified_dedups_entries -- --nocapture`
- `cargo test test_archive_stale_memories -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不扩展 event schema，也不新增 event type。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
