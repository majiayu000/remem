# Step 23 - Split eval_local module

## Why

`src/eval_local.rs` 当前同时包含评估报告类型、`Display` 渲染、总体评分、以及 dedup / project leak / title quality / self-retrieval 四类检查逻辑，达到 355 行，已经超过项目单文件 200 行限制。继续调整本地评估项时，影响范围不够聚焦。

本步只做结构拆分，不改变 `run_eval(conn)` 的对外接口，也不改变 `remem eval-local` 的输出语义。

## Scope

- 保持 `crate::eval_local::run_eval` 的公开接口不变
- 保持 `EvalReport` 与各子报告结构的字段不变
- 将 `src/eval_local.rs` 拆为报告类型、展示、运行入口和检查子模块
- 新增 `eval_local` 的最小回归测试
- 不修改 `src/cli/actions/eval.rs` 的调用方式

## Module layout

- `src/eval_local.rs`
  - 模块声明与 `pub use`
- `src/eval_local/types.rs`
  - `EvalReport`
  - `DedupReport`
  - `ProjectLeakReport`
  - `TitleQualityReport`
  - `SelfRetrievalReport`
- `src/eval_local/display.rs`
  - `Display for EvalReport`
  - `EvalReport::overall_score`
- `src/eval_local/run.rs`
  - `run_eval`
- `src/eval_local/dedup.rs`
  - `check_dedup`
- `src/eval_local/project_leak.rs`
  - `check_project_leak`
- `src/eval_local/title_quality.rs`
  - `check_title_quality`
- `src/eval_local/self_retrieval.rs`
  - `check_self_retrieval`
- `src/eval_local/tests.rs`
  - eval-local 回归测试

## Public interface invariants

- `run_eval(conn)` 继续只统计 active memories
- overall score 的权重保持不变：dedup 30%、project leak 25%、title quality 15%、self-retrieval 30%
- dedup 继续使用规范化内容前 200 字符做 hash 聚类
- project leak 继续取 active memory 最多的前 5 个项目、每项目取最多 3 个实体做检查
- title quality 继续统计 bullet-prefixed title 与超长 title
- self-retrieval 继续抽取最近 20 条、按标题关键词回搜

## Validation

定向测试：
- `cargo test eval_local_empty_db_reports_zeroes -- --nocapture`
- `cargo test eval_report_display_includes_overall_score -- --nocapture`
- `cargo test bench_topic_key_dedup -- --nocapture`
- `cargo test search_with_project_filter -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
