# Step 15 - Split workstream module

## Why

`src/workstream.rs` 目前同时承载类型定义、查询、模糊匹配、写入更新、生命周期迁移和全部测试，文件达到 581 行，已经超过项目单文件 200 行限制，也让后续调整 workstream 行为时难以控制影响面。

本步只做结构拆分，不改变公开接口和业务语义。

## Scope

- 保持 `crate::workstream::*` 的现有公开接口不变
- 将 `src/workstream.rs` 拆为更小的子模块
- 将 workstream 测试按职责迁移到独立测试子模块
- 不修改 `context`、`mcp` 等调用方的使用方式
- 不引入新功能，不改数据库 schema

## Module layout

- `src/workstream.rs`
  - 只保留模块声明与 `pub use`
- `src/workstream/types.rs`
  - `WorkStreamStatus`
  - `WorkStream`
  - `ParsedWorkStream`
- `src/workstream/query.rs`
  - `query_active_workstreams`
  - `query_workstreams`
  - `map_workstream_row`
- `src/workstream/matcher.rs`
  - `find_matching_workstream`
- `src/workstream/write.rs`
  - `upsert_workstream`
  - `update_workstream_manual`
- `src/workstream/lifecycle.rs`
  - `auto_pause_inactive`
  - `auto_abandon_inactive`
- `src/workstream/tests.rs`
  - 测试模块入口
- `src/workstream/tests/support.rs`
  - workstream 测试 schema helper
- `src/workstream/tests/query.rs`
  - 查询与匹配测试
- `src/workstream/tests/write.rs`
  - upsert 与手动更新测试
- `src/workstream/tests/lifecycle.rs`
  - 自动暂停/废弃测试

## Public interface invariants

- 继续通过 `crate::workstream` 暴露：
  - `WorkStreamStatus`
  - `WorkStream`
  - `ParsedWorkStream`
  - `query_active_workstreams`
  - `query_workstreams`
  - `find_matching_workstream`
  - `upsert_workstream`
  - `update_workstream_manual`
  - `auto_pause_inactive`
  - `auto_abandon_inactive`
- 调用方无需改 import 路径
- SQL、匹配规则、状态转换和返回值语义保持不变

## Validation

定向测试：
- `cargo test test_upsert_creates_new -- --nocapture`
- `cargo test test_upsert_updates_existing -- --nocapture`
- `cargo test test_fuzzy_match -- --nocapture`
- `cargo test test_update_workstream_manual -- --nocapture`
- `cargo test test_auto_pause_after_7_days -- --nocapture`
- `cargo test test_auto_abandon_after_30_days -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt --all` 再次带出既有的无关格式化噪音文件，只恢复那些无行为变更的格式差异，不把它们带进本次提交。
