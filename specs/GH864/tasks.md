# Task Plan

## Linked Issue

GH-864

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Maintainer approval: GH-864 issue comment dated 2026-07-17, after review of PR #873

## 实现任务

- [x] `SP864-T1` Owner: implementation agent; Done when: transcript evidence 截断稳定且原有校验不放宽； Verify: 见 SP864-T1。
- [x] `SP864-T2` Owner: implementation agent; Done when: exact range 路径原子且不改变 sibling ranges； Verify: 见 SP864-T2。
- [x] `SP864-T3` Owner: implementation agent; Done when: 所有 Git metadata 进程组在 deadline 后有界终止并执行统一 child/reader cleanup； Verify: 见 SP864-T3。
- [x] `SP864-T4` Owner: implementation agent; Done when: topic_key 使用共享 slug 规则且空结果 fail closed； Verify: 见 SP864-T4。
- [x] `SP864-T5` Owner: release implementation agent; Done when: patch release 表面和 changelog 同步； Verify: 见 SP864-T5。
- [x] `SP864-T8` Owner: implementation agent; Done when: quarantined range 只能经显式 exact-ID 确认恢复，默认与 batch 行为不变； Verify: 见 SP864-T8。
- [ ] `SP864-T9` Owner: implementation agent; Done when: archived quarantine 只能经 exact 双确认恢复，且 exact worker 只处理指定 task/profile； Verify: 见 SP864-T9。

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
- Files: `src/cli/types.rs`, `src/cli/actions/pending.rs`, `src/cli/tests_maintenance.rs`, `src/db/extraction_replay.rs`, `src/db/extraction/retry_regression_tests.rs`, `docs/specs/failure-lifecycle/PRODUCT.md`, `docs/specs/failure-lifecycle/TECH.md`
- Covers: B-007, B-008, B-009, B-010, B-011
- Done when:
  - list/retry/quarantine 接受正数 `--id`，并在解析阶段只拒绝用户显式提供的 `--project`、`--limit`；
    `--id`-only 不受隐式默认 limit 影响。
  - exact list 通过只读 ID 查询返回 terminal `replayed` range 及 replay task id/status/attempt/error；不存在
    的 ID 报错，不回退到 batch，JSON 不暴露 captured payload 或 provider secret。
  - dry-run 和执行路径复用同一 retryable predicate；执行在单事务内重新验证目标。
  - exact retry/quarantine 只改变目标 range，竞争或非法状态不退回批量选择。
  - 无 `--id` 的 oldest-first、project、limit、事务和返回计数语义保持不变。
  - failure-lifecycle PRODUCT/TECH 同步 exact list/retry/quarantine、terminal evidence 与 sibling 隔离合同。
- Verify:
  - `cargo test pending_exact_range_id_accepts_implicit_default_limit --locked`
  - `cargo test pending_exact_range_id_conflicts_with_batch_filters --locked`
  - `cargo test exact_range_list_includes_replayed_task_evidence --locked`
  - `cargo test exact_replay_range_operations_do_not_mutate_sibling_ranges --locked`
  - `cargo test extraction_replay --locked`

### SP864-T3 — 为 Git metadata probe 增加有界生命周期

- Owner: implementation agent
- Dependencies: none
- Files: `src/git_util.rs`, `src/db/core.rs`；`src/git_evidence.rs` 为必须审计但预计无需修改的真实 caller
- Covers: B-004, B-005, B-006
- Done when:
  - soft branch/commit probe、`resolve_toplevel` 和真实 `resolve_commit_metadata` 命令全部共用 2 秒 executor。
  - 每个 Git probe 使用独立 Unix process group；timeout/lifecycle error 先 TERM 整组、在有界 grace 后 KILL
    整组，并 reap direct child。
  - stdout/stderr 在 child 运行期间由独立 reader 并发 drain；超过 OS pipe buffer 的合法输出不会被误判
    为 timeout。reader 通过 channel 报告 completion，主线程只在 completion 后 join；cleanup deadline
    内未完成则终止残余进程组并返回 lifecycle error；即使 direct child 已正常退出也不得无界 join。
  - required metadata 使用保留错误的 `Result<PathBuf>` toplevel 路径，timeout/lifecycle error 不得经由
    soft `Option` helper 丢失，且错误保留 argv 类别和 cwd。
  - timeout 路径可靠 kill/reap；spawn 后 `try_wait`/wait/kill/reap 错误先尝试 bounded best-effort cleanup，
    并以 error 级别记录 argv 类别、cwd 和 cleanup 结果。
  - soft 与 required Git 调用分别保留 None 与 contextual error 语义，且无 shell 解释路径。
  - 维护者完成人工安全审查，确认固定 executable/argv、deadline、真实 caller 接线和 child 回收边界。
- Verify:
  - `cargo test command_output_with_timeout_kills_process_group --locked`
  - `cargo test command_output_with_timeout_cleans_up_after_poll_error --locked`
  - `cargo test command_output_with_timeout_drains_large_output --locked`
  - `cargo test command_output_with_timeout_bounds_reader_completion --locked`
  - `cargo test required_toplevel_preserves_timeout_context --locked`
  - `cargo test git_metadata_commands_use_bounded_executor --locked`
  - `cargo clippy --all-targets -- -D warnings`
  - 人工审查 `src/git_util.rs`、`src/git_evidence.rs`、`src/db/core.rs` 的 subprocess 生命周期和日志上下文

### SP864-T4 — 统一 topic_key 规范化

- Owner: implementation agent
- Dependencies: none
- Files: `src/session_rollup/parse.rs`
- Covers: B-012, B-013, B-014
- Done when:
  - parser 仅原样保留符合旧 grammar 且至少包含一个 ASCII 字母或数字的 key；其它非空原值调用
    `slugify_for_topic(..., 96)`。
  - `v0.2-release-audit` 稳定得到 `v0-2-release-audit`，重复标点按共享规则折叠。
  - 缺失、trim 后为空或规范化后为空继续返回明确错误。
  - 符合旧 grammar 的合法 kebab-case/snake_case key 走兼容快路径并原样保留。
- Verify:
  - `cargo test normalizes_version_punctuation_in_topic_key --locked`
  - `cargo test rejects_topic_key_that_normalizes_to_empty --locked`
  - `cargo test preserves_existing_snake_case_topic_key --locked`
  - `cargo test rejects_punctuation_only_topic_key --locked`
  - `cargo test session_rollup --locked`

### SP864-T5 — 同步 patch release 表面

- Owner: release implementation agent
- Dependencies: SP864-T1, SP864-T2, SP864-T3, SP864-T4
- Files: `README.md`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `plugins/remem/.codex-plugin/plugin.json`, `plugins/remem/runtimes/remem-releases.json`, `npm/remem/package.json`, `server.json`
- Covers: B-015
- Done when:
  - 所有发行版本面同步到同一 patch 版本，changelog 准确列出四项修复。
  - README 的 pending 命令区记录 exact-ID list/retry/quarantine 示例和 terminal JSON evidence 用法。
  - 发布说明不声称代码合并会自动恢复 range 308。
- Verify:
  - `python3 scripts/ci/check_plugin_version_sync.py`
  - `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`

### SP864-T8 — 显式确认 quarantined exact retry

- Owner: implementation agent
- Dependencies: SP864-T2
- Files: `src/cli/types.rs`, `src/cli/actions/pending.rs`, `src/cli/tests_maintenance.rs`, `src/db/extraction_replay.rs`, `src/db/extraction/retry_regression_tests.rs`, `README.md`, `docs/specs/README.md`, `docs/specs/failure-lifecycle/PRODUCT.md`, `docs/specs/failure-lifecycle/TECH.md`, release version surfaces
- Covers: B-016, B-017, B-018
- Done when:
  - `--acknowledge-quarantine` 在解析阶段要求正数 `--id`，且只存在于 exact retry。
  - 无确认的 exact retry 与所有 batch retry 仍只选择 `pending|failed`；显式确认只把目标
    `quarantined` range 纳入同一未归档、无 active replay task 的 predicate。
  - dry-run 和写事务接收同一确认值；事务重新验证后只 requeue 目标 range，失败、重复或竞争不改变 sibling。
  - README、failure-lifecycle PRODUCT/TECH、规格索引、changelog 与 patch 版本面同步。
- Verify:
  - `cargo test pending_quarantine_acknowledgement_requires_exact_id --locked`
  - `cargo test acknowledged_quarantined_range_preserves_other_illegal_state_rejections --locked`
  - `cargo test acknowledged_quarantined_range_retry_is_exact_and_batch_compatible --locked`
  - `python3 scripts/ci/check_plugin_version_sync.py`
  - `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`

### SP864-T9 — Archived quarantine escape hatch 与 exact worker

- Owner: implementation agent
- Dependencies: SP864-T8
- Files: `src/cli/types.rs`, `src/cli/worker_types.rs`, `src/cli/mod.rs`, `src/cli/dispatch.rs`, `src/cli/actions/pending.rs`, `src/cli/tests_maintenance.rs`, `src/db/extraction_replay.rs`, `src/db/extraction/retry_regression_tests.rs`, `src/db/extraction/lifecycle.rs`, `src/db/extraction/tests.rs`, `src/extraction_worker.rs`, `src/worker.rs`, README、failure-lifecycle contract 与 release version surfaces
- Covers: B-020, B-021, B-022
- Done when:
  - `--include-archived` 在解析阶段只允许正数 exact `--id` 与 `--dry-run`；archived quarantine 同时要求
    `--acknowledge-quarantine`，pending 命令不提供 archived 写路径，普通 exact/batch 集合不变。
  - exact worker 的一个事务用同一双确认 predicate 复验，清除目标 archive marker、只 requeue 目标并立即
    exact-claim；事务不能提交 daemon 可见的未 claim pending task，失败、竞争与 sibling 均无副作用。
  - `worker --once --replay-range-id <id> --acknowledge-quarantine --include-archived --profile <name>` 在任何
    写入前验证 profile 并取得 singleton，持锁原子 requeue+claim 后 process 指定 range 的 task 一次；ID
    claim 保留普通 retry readiness。非成功结果和 exact owner 过期 lease 均恢复 archived quarantine，不能
    进入 daemon 默认 profile 路径；不执行全局 maintenance、其它 extraction、job 或 backfill。
  - CLI 类型拆分后受影响文件保持低于 800 行；版本面同步到 0.6.4。
- Verify:
  - `cargo test archived_quarantined_range_requires_dual_exact_acknowledgement --locked`
  - `cargo test exact_extraction_task_claim_preserves_retry_readiness --locked`
  - `cargo test worker_exact_range_locks_before_requeue_and_processes_only_target --locked`
  - `python3 scripts/ci/check_plugin_version_sync.py`
  - `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`

## 并行拆分

- SP864-T1、SP864-T2、SP864-T3、SP864-T4 可并行；各任务仅修改其 `Files` 中的互斥文件。
- SP864-T5 必须等待四个实现任务完成，避免多个任务同时修改版本文件。
- SP864-T8 在 SP864-T2 后串行执行；它与 T2/T5 重叠 CLI、DB、文档和版本文件，不得并行写入。
- SP864-T9 在 SP864-T8 后串行执行；它拆分 CLI worker 参数并触及同一 replay DB 文件，不得并行写入。
- 若使用并行 agent，每个 agent 必须只拥有一个上述实现任务；共享验证与发行文件由串行收口 owner 处理。

## 验证任务

- [ ] `SP864-T6` Owner: verification agent; Done when: focused、全量、SpecRail 与 PR preflight 全部通过； Verify: 见 SP864-T6。
- [ ] `SP864-T7` Owner: release operator; Done when: 已认证 Claude profile 下完成 range 308 exact retry 并记录证据； Verify: 见 SP864-T7。

### SP864-T6 — 完整确定性验证与 PR preflight

- Owner: verification agent
- Dependencies: SP864-T1, SP864-T2, SP864-T3, SP864-T4, SP864-T5, SP864-T8, SP864-T9
- Covers: B-001, B-002, B-003, B-004, B-005, B-006, B-007, B-008, B-009, B-010, B-011, B-012, B-013, B-014, B-015, B-016, B-017, B-018, B-020, B-021, B-022
- Done when:
  - product-to-test mapping 中的 focused tests 全部通过。
  - Rust、Node、版本同步、版本 bump、diff 和 SpecRail packet 检查全部通过。
  - 使用实现 PR 的实际 body 运行完整 `check_pr_preflight.py`（不得使用 `--fast` 或跳过 body checks）并通过。
  - PR head、CI、review threads 和人工 merge 授权通过新鲜 PR gate；不得用历史输出替代。
- Verify:
  - `cargo fmt --check`
  - `cargo check --locked`
  - `cargo test --locked --quiet`
  - `cargo clippy --all-targets -- -D warnings`
  - `node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/request-security.test.js plugins/remem/apps/remem/server.test.js npm/remem/scripts/install.test.js`
  - `python3 scripts/ci/check_plugin_version_sync.py`
  - `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`
  - `python3 scripts/ci/check_pr_preflight.py --base <base-sha> --head HEAD --pr-body-file <body-file>`
  - `python3 checks/check_workflow.py --repo .`
  - `python3 checks/check_workflow.py --repo . --spec-dir=specs/GH864`
  - `git diff --check`

### SP864-T7 — 真实 range 308 运维收口

- Owner: release operator
- Dependencies: SP864-T6, SP864-T9, 可用且已认证的 Claude profile
- Covers: B-015, B-019, B-020, B-021, B-022
- Done when:
  - 记录成功的 Claude profile live check，先执行带 quarantine/archive 双确认的 exact-ID dry-run，再由持锁
    exact worker 原子完成同一 range 的 retry+claim 并处理；锁被 daemon 持有时在写入前失败，非成功时保持
    archived quarantine 而不是留下 daemon 可 claim 的 pending task。
  - 等待 worker 终态后运行 exact list；GH-864 记录 range 308 的精确 range/task ID、状态、attempt/error
    以及对应 replay task 的已脱敏 provider/profile 日志证据。
  - 失败时保留 issue open 并记录可诊断错误，不批量重试 sibling ranges。
- Verify:
  - `remem model test --profile claude --live`
  - `remem pending retry-extraction-ranges --id 308 --acknowledge-quarantine --include-archived --dry-run`
  - `remem worker --once --replay-range-id 308 --acknowledge-quarantine --include-archived --profile claude`
  - `remem pending list-extraction-ranges --id 308 --json`（记录 worker 终态）
  - 按 exact list 返回的 `replay_task_id` 关联 worker 日志，记录 provider/profile 与 terminal outcome（脱敏）

## Handoff Notes

- 已批准的 spec 不等于最终实现审批；实现 PR 仍需新鲜 CI、独立 review、全部线程解决和人工 merge 授权。
- Git subprocess 与 exact-range transaction 是合并前的强制人工安全/正确性审查点。
- range 308 的真实恢复依赖外部 Claude profile，不能由 fixture、模拟输出或批量 retry 代替。
- Product invariant set: B-001..B-022。
- Task coverage union: B-001..B-022；无缺失项。
