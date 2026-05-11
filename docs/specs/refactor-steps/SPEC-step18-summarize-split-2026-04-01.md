# Step 18 - Split summarize module

## Why

`src/summarize.rs` 之前同时承载 stop hook 输入读取、summary XML 解析、summary job 执行、compress job 执行和常量定义，文件达到 501 行，已经超过项目单文件 200 行限制，也使得后续修改 summary 或 compress 流程时难以控制影响面。

本步只做结构拆分，不改变 CLI/worker 入口、summary prompt、compress prompt 或现有数据库写入语义。

## Scope

- 保持 `crate::summarize::summarize`、`process_summary_job_input`、`process_compress_job`、`parse_summary` 对外接口不变
- 将 `src/summarize.rs` 拆为按职责分离的子模块
- 新增 parser 与 transcript 提取回归测试
- 不修改 `worker.rs`、`cli/mod.rs` 的调用方式
- 不改变 summary/compress 的 AI 输入输出契约

## Module layout

- `src/summarize.rs`
  - 模块声明与 `pub use`
- `src/summarize/constants.rs`
  - prompt 路径与 summarize/compress 常量
- `src/summarize/input.rs`
  - `SummarizeInput`
  - `hash_message`
  - `extract_last_assistant_message`
  - `read_stdin_with_timeout`
- `src/summarize/parse.rs`
  - `ParsedSummary`
  - `parse_summary`
- `src/summarize/summary_job.rs`
  - summary job 子模块组织与 re-export
- `src/summarize/summary_job/hook.rs`
  - `summarize`
  - enqueue + worker spawn
- `src/summarize/summary_job/process.rs`
  - `process_summary_job_input`
  - AI 调用与输入准备
- `src/summarize/summary_job/persist.rs`
  - existing summary context 构建
  - summary finalize/promotion/native sync
- `src/summarize/compress.rs`
  - `process_compress_job`
  - compress pipeline
- `src/summarize/tests.rs`
  - 测试入口
- `src/summarize/tests/parse.rs`
  - `<summary>` 解析测试
- `src/summarize/tests/input.rs`
  - transcript 提取测试

## Public interface invariants

- `summarize()` 仍然只负责读 stdin、排队 summary/compress 任务并启动一次 worker
- `process_summary_job_input()` 仍然执行 cooldown、去重、AI summarize、summary finalize、memory promote、Claude memory sync
- `process_compress_job()` 仍然调用压缩流程，不改变阈值和批量大小
- `parse_summary()` 继续基于 `<summary>...</summary>` 和 `memory_format::extract_field` 解析
- `<skip_summary` 仍然直接跳过 summary 解析/处理

## Validation

定向测试：
- `cargo test parse_summary_extracts_fields -- --nocapture`
- `cargo test parse_summary_returns_none_for_skip_marker -- --nocapture`
- `cargo test extract_last_assistant_message_skips_malformed_lines -- --nocapture`
- `cargo test bench_summary_parse_full -- --nocapture`
- `cargo test bench_summary_parse_skip -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 若 `cargo fmt` 再次带出已知的无关格式化噪音文件，只恢复这些无行为变化的差异，不将其纳入本批提交。
