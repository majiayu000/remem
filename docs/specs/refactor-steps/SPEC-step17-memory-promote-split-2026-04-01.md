# Step 17 - Split memory_promote module

## Why

`src/memory_promote.rs` 当前同时承载 topic key/slug 生成、标题与内容格式化、summary promote 主流程以及全部测试，文件达到 563 行，超过项目单文件 200 行限制，也让后续修改 promote 规则时难以控制影响范围。

本步只做结构拆分，不改变公开接口和 promote 语义。

## Scope

- 保持 `crate::memory_promote::slugify_for_topic` 和 `crate::memory_promote::promote_summary_to_memories` 对外接口不变
- 将 `src/memory_promote.rs` 拆为多个子模块
- 将测试拆到独立测试子模块
- 不修改 `src/memory.rs` 的 re-export 方式
- 不改变 dedup、title、content、split 的行为规则

## Module layout

- `src/memory_promote.rs`
  - 模块声明
  - 对外 `pub use`
- `src/memory_promote/slug.rs`
  - `slugify_for_topic`
  - `slugify`
  - `content_hash`
- `src/memory_promote/format.rs`
  - `build_title`
  - `build_item_title`
  - `truncate_at_boundary`
  - `build_content`
  - `split_into_items`
  - 相关常量
- `src/memory_promote/promote.rs`
  - `promote_summary_to_memories`
  - 内部 promote helper
- `src/memory_promote/tests.rs`
  - 测试入口
- `src/memory_promote/tests/format.rs`
  - slug/split/title/content/truncate 测试
- `src/memory_promote/tests/promote.rs`
  - promote 与 dedup 测试

## Public interface invariants

- `slugify_for_topic(text, max_len)` 行为不变
- `promote_summary_to_memories(...)` 返回值和 dedup 语义不变
- decision / discovery / preference 的 topic key 前缀不变
- content hash 仍基于标准化后的前 200 字符
- preference 仍走 `insert_memory_full(..., scope="global")`

## Validation

定向测试：
- `cargo test test_split_into_items_bullets -- --nocapture`
- `cargo test test_build_content_no_boilerplate -- --nocapture`
- `cargo test test_truncate_cjk_exact_boundary_panic_regression -- --nocapture`
- `cargo test test_promote_multi_decisions -- --nocapture`
- `cargo test test_cross_session_dedup -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
