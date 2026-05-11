## Goal

拆分 `src/preference.rs`，把 preference 的查询、渲染、CLI 操作和测试拆成独立模块，同时保持公开函数名与行为不变。

## Current State

- `src/preference.rs` 当前同时包含：
  - global preference 查询
  - CLAUDE.md 去重
  - context 渲染
  - CLI list/add/remove
  - tests

## Split Plan

- `src/preference.rs`
  - 只保留模块组织与 re-export
- `src/preference/query.rs`
  - `query_global_preferences`
- `src/preference/render.rs`
  - `dedup_with_claude_md`
  - `render_preferences`
- `src/preference/command.rs`
  - `list_preferences`
  - `add_preference`
  - `remove_preference`
- `src/preference/tests.rs`
  - preference 测试

## Constraints

- 不改公开函数名
- 不改 global preference 的阈值定义
- 不改 `render_preferences` 的输出标题和字符截断规则
- 不改 add/remove preference 的存储语义

## Non-Goals

- 不改 context 的整体输出结构
- 不顺手调整 topic_key 或 title 生成策略
- 不改 CLI 文案

## Verification

- 定向测试：
  - `cargo test test_render_preferences_empty -- --nocapture`
  - `cargo test test_render_preferences_with_data -- --nocapture`
  - `cargo test test_global_preferences_threshold -- --nocapture`
  - `cargo test test_dedup_with_claude_md -- --nocapture`
  - `cargo test test_add_and_remove_preference -- --nocapture`
- 完整验证：
  - `cargo check`
  - `cargo test`
