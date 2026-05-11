# Step 36 - Split dedup module

## Why

`src/dedup.rs` 当前同时包含 hash duplicate 查询、duplicate access 回写、三层 duplicate funnel 和内联测试。虽然文件还没有超过 200 行，但它已经承载了 observation 去重链路的核心逻辑，拆开后可以把候选查询、状态回写和总控判断分层，后续继续演进 vector/LLM 去重时会更稳。

本步只做结构拆分，不改变 `find_hash_duplicates()`、`mark_duplicate_accessed()`、`check_duplicate()` 的公开接口，也不改变当前 hash window、duplicate access 回写和返回 duplicate id 的语义。

## Scope

- 保持 `find_hash_duplicates()`、`mark_duplicate_accessed()`、`check_duplicate()` 对外接口不变
- 将 `src/dedup.rs` 拆为 `hash`、`access`、`funnel`、`tests` 子模块
- 保持 15 分钟 hash dedup window 不变
- 保持 duplicate 命中后记录 info 日志并回写 `last_accessed_epoch` 的语义不变
- 新增最小回归测试，锁住 access 更新时间和 duplicate id 返回行为

## Module layout

- `src/dedup.rs`
  - 模块声明与 `pub use`
- `src/dedup/hash.rs`
  - `find_hash_duplicates`
- `src/dedup/access.rs`
  - `mark_duplicate_accessed`
- `src/dedup/funnel.rs`
  - `check_duplicate`
- `src/dedup/tests.rs`
  - hash dedup / accessed / funnel 回归测试

## Public interface invariants

- `find_hash_duplicates()` 继续只在给定时间窗口、同 project、`status='active'` 的 observation 上做 hash 比对
- `mark_duplicate_accessed()` 继续在 ids 非空时批量更新 `last_accessed_epoch`
- `check_duplicate()` 继续先做 hash dedup，命中时返回第一个 duplicate id，并更新命中的 rows
- vector/LLM 去重仍保持 TODO，不在本步实现

## Validation

定向测试：
- `cargo test test_hash_dedup_finds_exact_match -- --nocapture`
- `cargo test mark_duplicate_accessed_updates_timestamp -- --nocapture`
- `cargo test check_duplicate_returns_first_hash_duplicate -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 测试继续使用本模块内最小 observation schema，不扩展业务行为。
- 若 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
