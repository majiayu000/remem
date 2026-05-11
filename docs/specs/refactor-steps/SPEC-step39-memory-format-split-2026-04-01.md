# Step 39 - Split memory_format module

## Why

`src/memory_format.rs` 当前同时包含 observation 解析结构、XML 转义、字段抽取、数组抽取、observation XML 解析和内联测试。虽然文件不大，但职责已经混在一起，后续如果继续改 flush/compress 的 XML 协议，会让边界不够清楚。

本步只做结构拆分，不改变公开接口。额外把 `OBSERVATION_TYPES` 的真实来源收敛到已有 `db_models`，避免同一套 observation 类型维护两份。

## Scope

- 保持公开接口不变：
  - `OBSERVATION_TYPES`
  - `ParsedObservation`
  - `xml_escape_text()`
  - `xml_escape_attr()`
  - `extract_field()`
  - `parse_observations()`
- 将 `src/memory_format.rs` 拆为 `types`、`escape`、`extract`、`parse`、`tests` 子模块
- `OBSERVATION_TYPES` 改为复用 `db_models` 的单一来源，不改变对外可见路径
- 保持非法 observation type 回退到 `discovery`、并从 `concepts` 中移除同名 type 的语义不变
- 补一条 parse 行为回归测试

## Module layout

- `src/memory_format.rs`
  - 模块声明与 `pub use`
- `src/memory_format/types.rs`
  - `ParsedObservation`
- `src/memory_format/escape.rs`
  - `xml_escape_text` / `xml_escape_attr`
- `src/memory_format/extract.rs`
  - `extract_field`
- `src/memory_format/parse.rs`
  - `parse_observations`
- `src/memory_format/tests.rs`
  - XML escape / field extract / parse 回归测试

## Public interface invariants

- `xml_escape_text()` / `xml_escape_attr()` 保持现有字符转义规则
- `extract_field()` 继续从首个合法 open tag 后开始匹配 close tag
- `parse_observations()` 继续跳过不完整 observation 块
- 非法 `<type>` 继续回退到 `discovery`
- `concepts` 中与最终 `obs_type` 同名的项继续被移除

## Validation

定向测试：
- `cargo test extract_field_scans_from_open_tag -- --nocapture`
- `cargo test xml_escape_escapes_angle_and_amp -- --nocapture`
- `cargo test parse_observations_defaults_invalid_type_and_filters_type_concept -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不改 observation XML 协议，也不扩展可识别字段。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
