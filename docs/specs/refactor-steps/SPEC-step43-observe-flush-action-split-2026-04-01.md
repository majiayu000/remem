# Step 43 - Split observe_flush action module

## Why

`src/observe_flush/action.rs` 当前把 action batch flush 的全部职责堆在一个文件里：batch clone、上下文构造、AI 调用、timeout split、失败回写、observation 解析、persist、结果汇总。文件已经接近上限，后续无论是改 split 策略还是改结果汇总，都会让影响面难以把握。

本步只做结构拆分，不改变 action batch flush 的对外行为和重试语义。

## Scope

- 保持内部接口不变：
  - `ActionFlushOutcome`
  - `flush_action_batches()`
- 将 `src/observe_flush/action.rs` 拆为 `types`、`helpers`、`runner`、`tests` 子模块
- 保持 timeout split 条件不变：只有 timeout 且 batch 大于 `FLUSH_RETRY_MIN_BATCH_SIZE` 才尝试二分
- 保持 observation 标题收集逻辑不变：只收集非空 title
- 新增纯逻辑测试，锁住 split 和标题收集行为

## Module layout

- `src/observe_flush/action.rs`
  - 模块声明与 re-export
- `src/observe_flush/action/types.rs`
  - `ActionFlushOutcome`
- `src/observe_flush/action/helpers.rs`
  - batch clone / timeout split / title 收集 helper
- `src/observe_flush/action/runner.rs`
  - `flush_action_batches`
- `src/observe_flush/action/tests.rs`
  - split / title helper 回归测试

## Public interface invariants

- `flush_action_batches()` 继续对 timeout 批次做二分重试，对其他 AI 错误做 retry 标记并返回错误
- observation 为空时继续 fail pending rows
- `split_retries`、`total_observations`、`titles` 的统计语义不变

## Validation

定向测试：
- `cargo test split_timeout_range_splits_evenly_when_possible -- --nocapture`
- `cargo test collect_observation_titles_skips_missing_titles -- --nocapture`
- `cargo test build_existing_context_includes_observations_and_memories -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 本步不改 AI prompt、不改 persist 逻辑、不改 retry backoff 规则。
- 如果 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
