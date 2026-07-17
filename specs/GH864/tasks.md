# Task Plan

## Linked Issue

GH-864

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Maintainer approval: GH-864 issue comment dated 2026-07-17, after review of PR #873

## 实现任务

- [ ] `SP864-T1` Owner: implementation agent; Done when: transcript evidence 截断稳定且原有校验不放宽； Verify: 见 SP864-T1。
- [ ] `SP864-T2` Owner: implementation agent; Done when: exact range 路径原子且不改变 sibling ranges； Verify: 见 SP864-T2。
- [ ] `SP864-T3` Owner: implementation agent; Done when: Git probe 在 2 秒内终止并可靠回收 child； Verify: 见 SP864-T3。
- [ ] `SP864-T4` Owner: implementation agent; Done when: topic_key 使用共享 slug 规则且空结果 fail closed； Verify: 见 SP864-T4。
- [ ] `SP864-T5` Owner: release implementation agent; Done when: patch release 表面和 changelog 同步； Verify: 见 SP864-T5。

### SP864-T1 — 稳定 transcript evidence 截断

- Owner: implementation agent
- Dependencies: none
- Files: `src/session_rollup/transcript_evidence.rs`
- Covers: B-001, B-002, B-003
- Done when:
  - 单消息和总预算路径共用 UTF-8 安全截断、`trim_end` 和真实字节计数。
  - 空消息继续被丢弃，现有角色、range、脱敏和预算校验未放宽。
  - trailing-whitespace、UTF-8、空结果和总预算回归测试通过。
- Verify:
  - `cargo test per_message_budget_keeps_redaction_idempotent_at_whitespace_boundary --locked`
  - `cargo test total_budget_never_retains_empty_utf8_message --locked`
  - `cargo test session_rollup --locked`

### SP864-T2 — 增加 exact extraction range 操作

- Owner: implementation agent
- Dependencies: none
- Files: `src/cli/types.rs`, `src/cli/actions/pending.rs`, `src/cli/tests_maintenance.rs`, `src/db/extraction_replay.rs`, `src/db/extraction/retry_regression_tests.rs`
- Covers: B-007, B-008, B-009, B-010, B-011
- Done when:
  - retry/quarantine 接受正数 `--id`，并在解析阶段拒绝与 `--project`、`--limit` 组合。
  - dry-run 和执行路径复用同一 retryable predicate；执行在单事务内重新验证目标。
  - exact retry/quarantine 只改变目标 range，竞争或非法状态不退回批量选择。
  - 无 `--id` 的 oldest-first、project、limit、事务和返回计数语义保持不变。
- Verify:
  - `cargo test pending_exact_range_id_conflicts_with_batch_filters --locked`
  - `cargo test exact_replay_range_operations_do_not_mutate_sibling_ranges --locked`
  - `cargo test extraction_replay --locked`

### SP864-T3 — 为 Git metadata probe 增加有界生命周期

- Owner: implementation agent
- Dependencies: none
- Files: `src/db/core.rs`
- Covers: B-004, B-005, B-006
- Done when:
  - branch/commit probe 固定使用 argv 调用并在 2 秒内返回。
  - timeout 路径可靠 kill/reap；spawn、wait、kill、reap 错误以 error 级别记录 probe 类别和 cwd。
  - 正常非零退出继续返回无探测结果，且无 shell 解释路径。
  - 维护者完成人工安全审查，确认固定 executable/argv、deadline 和 child 回收边界。
- Verify:
  - `cargo test command_output_with_timeout_kills_long_running_child --locked`
  - `cargo clippy -- -D warnings`
  - 人工审查 `src/db/core.rs` 的 subprocess 生命周期和日志上下文

### SP864-T4 — 统一 topic_key 规范化

- Owner: implementation agent
- Dependencies: none
- Files: `src/session_rollup/parse.rs`
- Covers: B-012, B-013, B-014
- Done when:
  - parser 对非空原值调用 `slugify_for_topic(..., 96)`。
  - `v0.2-release-audit` 稳定得到 `v0-2-release-audit`，重复标点按共享规则折叠。
  - 缺失、trim 后为空或规范化后为空继续返回明确错误。
  - 既有合法 kebab-case/snake_case key 的语义身份保持稳定。
- Verify:
  - `cargo test normalizes_version_punctuation_in_topic_key --locked`
  - `cargo test rejects_topic_key_that_normalizes_to_empty --locked`
  - `cargo test session_rollup --locked`

### SP864-T5 — 同步 patch release 表面

- Owner: release implementation agent
- Dependencies: SP864-T1, SP864-T2, SP864-T3, SP864-T4
- Files: `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `plugins/remem/.codex-plugin/plugin.json`, `plugins/remem/runtimes/remem-releases.json`, `npm/remem/package.json`, `server.json`
- Covers: B-015
- Done when:
  - 所有发行版本面同步到同一 patch 版本，changelog 准确列出四项修复。
  - 发布说明不声称代码合并会自动恢复 range 308。
- Verify:
  - `python3 scripts/ci/check_plugin_version_sync.py`
  - `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`

## 并行拆分

- SP864-T1、SP864-T2、SP864-T3、SP864-T4 可并行；各任务仅修改其 `Files` 中的互斥文件。
- SP864-T5 必须等待四个实现任务完成，避免多个任务同时修改版本文件。
- 若使用并行 agent，每个 agent 必须只拥有一个上述实现任务；共享验证与发行文件由串行收口 owner 处理。

## 验证任务

- [ ] `SP864-T6` Owner: verification agent; Done when: focused、全量、SpecRail 与 PR preflight 全部通过； Verify: 见 SP864-T6。
- [ ] `SP864-T7` Owner: release operator; Done when: 已认证 Claude profile 下完成 range 308 exact retry 并记录证据； Verify: 见 SP864-T7。

### SP864-T6 — 完整确定性验证与 PR preflight

- Owner: verification agent
- Dependencies: SP864-T1, SP864-T2, SP864-T3, SP864-T4, SP864-T5
- Covers: B-001, B-002, B-003, B-004, B-005, B-006, B-007, B-008, B-009, B-010, B-011, B-012, B-013, B-014, B-015
- Done when:
  - product-to-test mapping 中的 focused tests 全部通过。
  - Rust、Node、版本同步、版本 bump、diff 和 SpecRail packet 检查全部通过。
  - PR head、CI、review threads 和人工 merge 授权通过新鲜 PR gate；不得用历史输出替代。
- Verify:
  - `cargo fmt --check`
  - `cargo check --locked`
  - `cargo test --locked --quiet`
  - `cargo clippy -- -D warnings`
  - `node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/server.test.js`
  - `python3 scripts/ci/check_plugin_version_sync.py`
  - `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`
  - `python3 checks/check_workflow.py --repo .`
  - `python3 checks/check_workflow.py --repo . --spec-dir=specs/GH864`
  - `git diff --check`

### SP864-T7 — 真实 range 308 运维收口

- Owner: release operator
- Dependencies: SP864-T6, 可用且已认证的 Claude profile
- Covers: B-015
- Done when:
  - 先执行 exact-ID dry-run 并记录目标状态，再执行非 dry-run retry。
  - GH-864 记录 range 308 的最终 range/task 状态和 provider 证据。
  - 失败时保留 issue open 并记录可诊断错误，不批量重试 sibling ranges。
- Verify:
  - `remem pending retry-extraction-ranges --id 308 --dry-run`
  - `remem pending retry-extraction-ranges --id 308`
  - `remem doctor --json`

## Handoff Notes

- 已批准的 spec 不等于最终实现审批；实现 PR 仍需新鲜 CI、独立 review、全部线程解决和人工 merge 授权。
- Git subprocess 与 exact-range transaction 是合并前的强制人工安全/正确性审查点。
- range 308 的真实恢复依赖外部 Claude profile，不能由 fixture、模拟输出或批量 retry 代替。
- Product invariant set: B-001..B-015。
- Task coverage union: B-001..B-015；无缺失项。
