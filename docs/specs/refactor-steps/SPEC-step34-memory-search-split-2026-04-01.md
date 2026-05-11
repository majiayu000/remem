# Step 34 - Split memory_search module

## Why

`src/memory_search.rs` 当前同时包含 project/branch filter helper、FTS 搜索和 LIKE fallback 搜索逻辑，达到 203 行，已经超过项目单文件 200 行限制。这个模块职责可以自然拆成过滤 helper、FTS 路径和 LIKE 路径，拆开后更方便后续继续调整 memory 检索语义。

本步只做结构拆分，不改变 `search_memories_fts()`、`search_memories_fts_filtered()`、`search_memories_like()`、`search_memories_like_filtered()` 的公开接口，也不改变当前 project/branch/status/memory_type 过滤和排序语义。

## Scope

- 保持 4 个公开搜索函数的签名和返回值不变
- 将 `src/memory_search.rs` 拆为 `filters`、`fts`、`like`、`tests` 子模块
- 保持 `push_project_filter()` 的现有公开可见性和行为不变
- 保持 FTS 使用 `bm25(...) * CASE WHEN memory_type IN ('decision','bugfix') THEN 1.5` 的排序语义不变
- 保持 LIKE fallback 的短 token 模糊匹配和 `updated_at_epoch DESC` 排序语义不变
- 新增最小回归测试，锁住 branch + include_inactive 的 SQL 过滤行为

## Module layout

- `src/memory_search.rs`
  - 模块声明与 `pub use`
- `src/memory_search/filters.rs`
  - `push_project_filter`
  - `push_branch_filter`
- `src/memory_search/fts.rs`
  - `search_memories_fts`
  - `search_memories_fts_filtered`
- `src/memory_search/like.rs`
  - `search_memories_like`
  - `search_memories_like_filtered`
- `src/memory_search/tests.rs`
  - FTS / LIKE 过滤回归测试

## Public interface invariants

- `search_memories_fts()` 继续只是 filtered 版本的简单包装
- `search_memories_like()` 继续只是 filtered 版本的简单包装
- `search_memories_*_filtered()` 继续默认在 `include_inactive=false` 时只返回 `status='active'`
- branch 过滤继续允许 `(branch = ? OR branch IS NULL)`
- `push_project_filter()` 继续把 exact project filter 下推到 SQL 条件中

## Validation

定向测试：
- `cargo test test_memory_fts_search -- --nocapture`
- `cargo test test_memory_like_fallback -- --nocapture`
- `cargo test search_memories_filtered_respects_branch_and_active_state -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 测试直接复用 `crate::memory::tests_helper::setup_memory_schema`，不手写第二份 schema。
- 若 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
