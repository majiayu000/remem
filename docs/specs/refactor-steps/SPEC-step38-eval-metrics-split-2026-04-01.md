# Step 38 - Split eval_metrics module

## Why

`src/eval_metrics.rs` 当前同时包含排序质量指标实现和全部内联测试。文件不大，但 `ndcg` 与 `precision/recall/hit/mrr` 属于两类不同职责，拆开后更利于后续增加评估指标或调整公式时做定向维护。

本步只做结构拆分，不改变任何公开函数名、参数或返回值语义。

## Scope

- 保持公开接口不变：
  - `ndcg_at_k()`
  - `reciprocal_rank()`
  - `precision_at_k()`
  - `recall_at_k()`
  - `hit_at_k()`
- 将 `src/eval_metrics.rs` 拆为 `ranking`、`retrieval`、`tests` 子模块
- 保持空输入、`k == 0`、无 relevant item 等边界语义不变
- 迁移并保留现有所有测试

## Module layout

- `src/eval_metrics.rs`
  - 模块声明与 `pub use`
- `src/eval_metrics/ranking.rs`
  - `ndcg_at_k`
- `src/eval_metrics/retrieval.rs`
  - `reciprocal_rank` / `precision_at_k` / `recall_at_k` / `hit_at_k`
- `src/eval_metrics/tests.rs`
  - 所有现有指标测试

## Public interface invariants

- `ndcg_at_k()` 继续在 `relevance` 为空或 `k == 0` 时返回 `0.0`
- `reciprocal_rank()` 继续返回首个 relevant result 的倒数排名
- `precision_at_k()` / `recall_at_k()` / `hit_at_k()` 保持现有 top-k 截断逻辑
- `recall_at_k()` 在 `relevant_ids` 为空时继续返回 `1.0`

## Validation

定向测试：
- `cargo test test_ndcg_perfect_ranking -- --nocapture`
- `cargo test test_reciprocal_rank_third -- --nocapture`
- `cargo test test_precision_at_k -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不新增指标，也不改 CLI eval 输出。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
