# Step 21 - Split query expand module

## Why

`src/query_expand.rs` 当前把同义词词典、CJK 判定、混合分词、CJK 最大正向匹配、query expansion 和测试全部放在一个文件里，达到 379 行，已经超过项目单文件 200 行限制。后续继续调整中英文检索扩展时，修改影响面也不够聚焦。

本步只做结构拆分，不改变 `expand_query` 和 `core_tokens` 的现有语义，也不改变搜索链路的调用方式。

## Scope

- 保持 `crate::query_expand::expand_query` 和 `crate::query_expand::core_tokens` 的公开接口不变
- 将 `src/query_expand.rs` 拆为职责分离的子模块
- 保留现有中英文同义词词典、CJK 分词规则和去重顺序
- 将现有 query expand 测试迁移到模块化结构
- 不修改 `src/search/memory.rs` 的调用方式

## Module layout

- `src/query_expand.rs`
  - 模块声明与 `pub use`
- `src/query_expand/synonyms.rs`
  - `SYNONYMS`
- `src/query_expand/tokenize.rs`
  - `is_cjk`
  - `tokenize_mixed`
  - `segment_cjk`
- `src/query_expand/expand.rs`
  - `core_tokens`
  - `expand_query`
  - `add_with_synonyms`
- `src/query_expand/tests.rs`
  - query expand 与 tokenization 回归测试

## Public interface invariants

- `core_tokens(raw)` 继续只返回用户意图 token，不做同义词扩展
- `expand_query(raw)` 继续执行 mixed tokenization、CJK segmentation、同义词扩展和去重
- CJK 词典分词仍然优先尝试 4/3/2 长度匹配，多字符命中时优先返回分段结果
- 原始 token 在需要 exact matching 的场景下仍保留在 expanded 结果中

## Validation

定向测试：
- `cargo test expand_english_to_chinese -- --nocapture`
- `cargo test cjk_segmentation_cross_project_sharing -- --nocapture`
- `cargo test tokenize_mixed_test -- --nocapture`
- `cargo test search_mixed_chinese_english -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
