## Goal

拆分 `src/timeline.rs`，把 timeline 报告的统计查询、full 模式明细查询、渲染逻辑和测试拆成独立模块，同时保持 `timeline::generate_timeline_report` 的公开入口和 Markdown 输出结构不变。

## Current State

- `src/timeline.rs` 当前同时包含：
  - overview / type count / token economics 查询
  - monthly aggregation 查询
  - full 模式 recent timeline 查询
  - Markdown report 组装
  - 测试
- 文件体量接近 500 行，已经明显超出仓库的单文件限制

## Split Plan

- `src/timeline.rs`
  - 只保留模块组织与 re-export
- `src/timeline/types.rs`
  - `Overview`
  - `TypeCount`
  - `MonthRow`
  - `TokenEcon`
  - `RecentObservation`
- `src/timeline/summary.rs`
  - `query_overview`
  - `query_type_counts`
  - `query_token_economics`
- `src/timeline/detail.rs`
  - `query_monthly`
  - `query_recent_observations`
- `src/timeline/report.rs`
  - `generate_timeline_report`
  - 内部 Markdown 渲染 helper
- `src/timeline/tests.rs`
  - timeline 报告测试

## Constraints

- 不改公开入口：`timeline::generate_timeline_report`
- 不改输出章节结构：
  - `# Journey Into ...`
  - `## Overview`
  - `## Activity by Type`
  - `## Token Economics`
  - full 模式下的 `## Timeline (recent first)` 与 `## Monthly Breakdown`
- 不调整 SQL 口径，不顺手修改统计定义

## Non-Goals

- 不改 MCP timeline tool 的入参/出参
- 不改 doctor/status 统计口径
- 不顺手把 timeline 逻辑搬进 `db_query`

## Verification

- 定向测试：
  - `cargo test empty_project_produces_report -- --nocapture`
  - `cargo test summary_report_excludes_timeline -- --nocapture`
  - `cargo test full_report_includes_timeline_and_monthly -- --nocapture`
- 完整验证：
  - `cargo check`
  - `cargo test`
