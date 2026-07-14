# Task Plan

## Linked Issue

GH-818

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## 当前阻塞与前置门禁

截至 2026-07-15，GH-818 只有 `bug`、`review`、`p1`、`extraction`、`observability`
labels，没有 readiness label；也没有 maintainer `spec_approval` 证据。不得把本 spec packet
的存在视为批准。

本地 deterministic gate 必须使用 GitHub 当前事实生成的可信 evidence，不得用 `--state`
伪造 readiness：

```bash
python3 checks/github_issue_evidence.py --github-repo majiayu000/remem --issue 818 --json > /tmp/gh818-issue-evidence.json
python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem --issue 818 --json > /tmp/gh818-duplicate-evidence.json
python3 checks/route_gate.py --repo . --route implement --issue 818 --evidence /tmp/gh818-issue-evidence.json --duplicate-evidence /tmp/gh818-duplicate-evidence.json --json
```

当前可信 issue evidence 没有 readiness label，结果必须保持 `needs_human`。Issue body 中的
readiness/去重说明是有用背景，但不能替代可信 label/state 与 machine-readable duplicate
evidence。开始任何实现前必须全部满足：

- maintainer 设置实际 readiness state/label 为 `ready_to_implement`；
- maintainer 留下明确 `spec_approval`；
- 生成并审核 duplicate evidence：
  `python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem --issue 818 --json`；
- 重新运行 implement route gate，结果必须不再阻止 `implement`。

当前授权仅限 spec planning；以下 checkbox 不得在上述门禁满足前开始或标记完成。

## 实现任务

- [ ] `SP818-T1` — v069 migration 与 schema contract — Owner: migration lane agent；Done when: 见下；Verify: 见下
  - Owner: migration lane agent；human review required。
  - Exclusive files: `src/migrations/v069_job_queue_atomicity.sql`、`src/migrate/types.rs`、
    `src/migrate/run.rs`、
    `src/migrate/schema_drift/invariants.rs`、`src/migrate/tests_job_queue_atomicity.rs`、
    `src/migrate.rs`。
  - Dependencies: readiness label、`spec_approval`、duplicate evidence、implement route gate
    通过。
  - Covers: `B-004`, `B-005`, `B-006`, `B-009`, `B-010`, `B-013`, `B-014`。
  - Done when: v069 在一个 migration transaction 内退休 late active Summary、按 Tech Spec
    稳定选择 ordinary/Dream/CompileRules survivor；pending Dream 按
    `(created_at_epoch ASC,id ASC)` 重放 current profile predicate，processing Dream payload
    不改写；redundant active rows 变为可见 permanent failures，保留各自真实 `attempt_count`
    不变，并由 `failure_class='permanent'` 与 `next_retry_epoch=0` 阻止重试，不得把计数提升到
    `max_attempts` 伪造 exhausted；其既有截断 `last_error` 保持为主证据，在 2000-char 上限内
    先为完整、确定性的 migration duplicate marker 预留空间再
    截断/追加；只有原 error 为 NULL/empty 时才单独存 marker。marker 包含 duplicate/canonical
    ids、identity kind、manual-review flag，migration 日志不输出原 error/payload。验证零重复后
    创建三个 partial UNIQUE indexes；v069 SQL 成功后、`mark_applied` 前，
    `run_post_migration_hook`（或等价 Rust hook）按 marker 统计各 identity kind 的 reconciled
    与 manual-review 数量并只记录计数；hook 查询/日志准备失败使整个 migration transaction
    回滚。terminal/archive history 不变，schema drift 声明完整，任一失败整体回滚。
  - Verify:
    - `cargo test --no-default-features v069_reconciles_active_job_duplicates_before_unique_indexes -- --nocapture`
    - `cargo test --no-default-features v069_replays_pending_dream_duplicates_with_current_profile_predicate -- --nocapture`
    - `cargo test --no-default-features v069_does_not_rewrite_processing_dream_payload -- --nocapture`
    - `cargo test --no-default-features v069_preserves_existing_duplicate_last_error_and_appends_marker -- --nocapture`
    - `cargo test --no-default-features v069_truncates_near_limit_duplicate_last_error_without_losing_marker -- --nocapture`
    - `cargo test --no-default-features v069_preserves_redundant_active_attempt_count_without_reporting_exhaustion -- --nocapture`
    - `cargo test --no-default-features v069_preserves_terminal_job_history_and_is_idempotent -- --nocapture`
    - `cargo test --no-default-features v069_post_migration_hook_logs_conflict_counts_without_payload -- --nocapture`
    - `cargo test --no-default-features job_queue_atomicity_migration_rolls_back_all_changes_on_validation_error -- --nocapture`
    - `cargo test --no-default-features validate_schema_invariants_is_clean_after_current_migrations -- --nocapture`

- [ ] `SP818-T2` — lease-owned state CAS 与 collision-aware release — Owner: state lane agent；Done when: 见下；Verify: 见下
  - Owner: state lane agent；human review required。
  - Exclusive files: `src/db/job/state.rs`、`src/db/job/tests.rs`。在 T2 完成前，其他 lane 不得
    写 `src/db/job/tests.rs`。
  - Dependencies: readiness label、`spec_approval`、duplicate evidence、implement route gate
    通过。
  - Covers: `B-001`, `B-002`, `B-003`, `B-006`, `B-012`, `B-014`。
  - Done when: done/retry/exhausted/permanent paths 全部使用 current processing + expected owner
    + unexpired lease 的 same-transaction CAS；恰好一行才成功；zero-row/missing diagnostics
    在同一 write transaction 中读取；拒绝路径所有持久化字段不变；attempt 分支无
    read-update TOCTOU。CompileRules retry/expired release 逐行处理：无 successor 时 guarded
    回到 pending，有 successor 时保留它为唯一 pending canonical、应用 retry ready time/priority
    并把 predecessor 留作 permanent failure；worker retry collision 写入本次真实失败对应的
    `next_attempt=attempt_count+1`，expired-lease collision 不代表一次新执行，保持当前计数；两者
    都不得写成 `max_attempts`。`failure_class='permanent'` 与 `next_retry_epoch=0` 已足以阻止再次
    auto-retry。原始或既有截断 `last_error` 是主证据，2000-char 内确定性追加 canonical marker。
    state API 返回结构化
    coalesced result 供 T4 worker 消费；T2 不直接输出原错误日志。一次 collision 不得触发
    UNIQUE error、反复 auto-retry 或阻止其他 expired jobs recovery。
  - Verify:
    - `cargo test --no-default-features lease_owned_job_transitions_require_current_unexpired_lease -- --nocapture`
    - `cargo test --no-default-features rejected_job_transition_preserves_every_persisted_field -- --nocapture`
    - `cargo test --no-default-features job_transition_error_reports_expected_and_current_lease -- --nocapture`
    - `cargo test --no-default-features compile_rules_retry_collision_coalesces_to_pending_successor -- --nocapture`
    - `cargo test --no-default-features compile_rules_retry_collision_preserves_original_error_with_bounded_marker -- --nocapture`
    - `cargo test --no-default-features release_expired_compile_rules_collision_preserves_unrelated_job_progress -- --nocapture`

- [ ] `SP818-T3` — atomic enqueue、三类 identity 与 claim eligibility — Owner: enqueue lane agent；Done when: 见下；Verify: 见下
  - Owner: enqueue lane agent；T2 后接管 `src/db/job/tests.rs`。
  - Exclusive files: `src/db/job/enqueue.rs`、`src/db/job/claim.rs`、`src/db/job.rs`、
    `src/db/job/tests.rs`、`src/session_rollup/side_effects.rs`。其他调用点仅在编译证明必须适配
    时先更新 ownership 表，不得并发写入。
  - Dependencies: `SP818-T1`, `SP818-T2`；`src/db/job/tests.rs` ownership 已从 T2 移交。
  - Covers: `B-004`, `B-005`, `B-006`, `B-007`, `B-008`, `B-013`, `B-014`。
  - Done when: public wrapper 与 transaction-scoped core 不产生 nested transaction；ordinary
    NULL-session、project-wide Dream、CompileRules state slots 与 v069 indexes 一致；所有
    caller 获得 persisted canonical id；Dream disposition/profile/priority/cooldown 保持；
    Summary enqueue 明确拒绝；claim query 与 conditional update 都跳过“同 project predecessor
    仍 processing”的 CompileRules successor，并继续 claim 全局顺序中的下一条 eligible
    unrelated job；至少三条真实 WAL/independent-connection barrier tests 通过。
  - Verify:
    - `cargo test --no-default-features enqueue_job_two_wal_connections_coalesce_ordinary_identity -- --nocapture`
    - `cargo test --no-default-features dream_two_wal_connections_coalesce_across_hosts -- --nocapture`
    - `cargo test --no-default-features compile_rules_two_wal_connections_share_one_pending_successor -- --nocapture`
    - `cargo test --no-default-features compile_rules_two_wal_connections_create_one_initial_pending -- --nocapture`
    - `cargo test --no-default-features ordinary_job_identity_normalizes_null_session_and_allows_terminal_history -- --nocapture`
    - `cargo test --no-default-features maybe_enqueue_dream_job_upgrades_pending_payload_for_profile_override -- --nocapture`
    - `cargo test --no-default-features maybe_enqueue_dream_job_skips_recent_done_job -- --nocapture`
    - `cargo test --no-default-features claim_next_job_skips_compile_rules_successor_while_predecessor_processing -- --nocapture`
    - `cargo test --no-default-features claim_next_job_continues_to_unrelated_eligible_job -- --nocapture`

- [ ] `SP818-T4` — worker、observability、failure contract 与 legacy fixtures — Owner: observability lane agent；Done when: 见下；Verify: 见下
  - Owner: observability lane agent。
  - Exclusive files: `src/worker.rs`、`src/worker/tests.rs`、`src/db/query/stats.rs`、
    `src/db/query/stats/tests.rs`、`src/doctor/database.rs`、`src/doctor/tests.rs`、
    `src/cli/actions/query/status.rs`、`src/cli/actions/query/status/types.rs`、
    `src/cli/actions/query/status/tests.rs`、`src/db/failure_lifecycle/maintenance.rs`、
    `src/db/failure_lifecycle/tests.rs`、`docs/specs/failure-lifecycle/PRODUCT.md`、
    `docs/specs/failure-lifecycle/TECH.md`。
  - Dependencies: `SP818-T2`, `SP818-T3`；必须在 T3 identity classifier、claim 与 Summary
    rejection API 稳定且 `src/db/job/tests.rs` ownership 已留在 T3 后开始，不得触碰 T2/T3
    exclusive files。
  - Covers: `B-004`, `B-005`, `B-006`, `B-009`, `B-010`, `B-011`, `B-012`, `B-013`,
    `B-014`。
  - Done when: transition error 以 error level 传播且无 done/retry success signal；worker 消费
    T2 的 structured coalesced result，只记录 safe marker、source/canonical ids 与 identity kind，
    不输出原始 retry error；shared stats
    仍以 persisted row 计算 processing/stuck/actionable failed；status text/JSON 与 doctor 使用
    同一口径；job auto-recovery 先取 bounded candidate list，再逐 row transaction/savepoint
    处理 retired Summary guard 以及 ordinary、Dream、CompileRules collision。candidate query
    排除 Summary，逐 row classifier 也必须在 generic no-active requeue 前保持任何意外 Summary
    source 的全部 terminal audit fields 不变，返回明确 retired/skipped result，且不计入
    requeued/coalesced；仅非 Summary 无 active 时 requeue source。普通 batch fixture 必须同时
    证明排除 Summary 后无关 ordinary recovery 仍前进；另用 transaction-scoped per-row helper 或
    等价 injected-candidate seam 精确覆盖 defense-in-depth guard。collision 时按 Tech Spec 收敛 canonical work，source 保持
    failed/auditable、保留真实 `attempt_count` 且不再
    重复 retry，既有截断 `last_error` 作为主证据并在 2000-char 内确定性追加 marker；一条 collision
    不回滚无关 recoveries，unexpected DB error 仍明确失败。migration conflicts 进入现有
    failure lifecycle；仅作 fixture 的 Summary enqueue 改为合适 non-retired type，真正的
    Summary retirement tests 继续直接构造历史 row；current failure contract 已记录新语义。
  - Verify:
    - `cargo test --no-default-features worker_transition_conflict_logs_error_without_done_or_retry_success -- --nocapture`
    - `cargo test --no-default-features worker_compile_rules_retry_collision_logs_safe_coalesced_result -- --nocapture`
    - `cargo test --no-default-features lease_transition_failure_remains_visible_in_status_and_doctor -- --nocapture`
    - `cargo test --no-default-features check_pending_queue_reports_shared_counts -- --nocapture`
    - `cargo test --no-default-features cli_status_renders_action_block_for_runtime_failures -- --nocapture`
    - `cargo test --no-default-features legacy_summary_upgrade_rejects_non_terminal_jobs -- --nocapture`
    - `cargo test --no-default-features worker_rejects_legacy_summary_job_without_retry -- --nocapture`
    - `cargo test --no-default-features failure_lifecycle_auto_recovery_excludes_legacy_summary_and_recovers_ordinary -- --nocapture`
    - `cargo test --no-default-features failure_lifecycle_per_row_guard_preserves_legacy_summary -- --nocapture`
    - `cargo test --no-default-features failure_lifecycle_auto_recovery_coalesces_mixed_active_identities_per_row -- --nocapture`
    - `cargo test --no-default-features failure_lifecycle_auto_recovery_preserves_source_error_and_does_not_repeat -- --nocapture`
    - `cargo test --no-default-features failure_lifecycle_auto_recovery_preserves_source_attempt_count -- --nocapture`
    - `cargo test --no-default-features failure_lifecycle -- --nocapture`

## 并行拆分

默认采用单 agent 串行顺序：`T1 → T2 → T3 → T4 → T5 → T6`，最容易满足 W-14。

若使用并行 lanes，只允许以下显式拓扑：

| Lane | 可并行阶段 | Exclusive ownership | Merge point |
| --- | --- | --- | --- |
| Migration | `SP818-T1` | 仅 T1 列出的 migration/schema files | T1 tests 通过后交给 T3 消费 schema contract |
| State | `SP818-T2` | `state.rs` + `src/db/job/tests.rs` | T2 完成后显式把 `tests.rs` ownership 移交 T3 |
| Claim/enqueue | `SP818-T3` | `claim.rs`、enqueue files，并从 T2 接收 `src/db/job/tests.rs` | T1/T2 均完成后启动；完成后冻结 identity/claim contract |
| Observability/recovery | `SP818-T4` | 仅 T4 列出的 worker/stats/doctor/status/failure lifecycle/docs files | T3 完成后启动；不得与 T2/T3 并行写 shared files |

`SP818-T3` 不与 T2 并行，因为二者顺序共享 `src/db/job/tests.rs`；T2 完成后必须显式移交给
T3，T4 不拥有该文件。`SP818-T4` 在 T3 后开始，独占
`src/db/failure_lifecycle/maintenance.rs` 与 `src/db/failure_lifecycle/tests.rs`。任何未列出的
shared file 一旦需要修改，先暂停对应 lanes、更新 ownership 和 dependencies，再继续；禁止
两个 agent 同时写同一文件。

## 验证任务

- [ ] `SP818-T5` — 全量 deterministic verification — Owner: integration agent；Done when: 见下；Verify: 见下
  - Owner: integration agent（单一 owner）。
  - Exclusive files: none；本任务只验证，发现失败时退回拥有该文件的 T1–T4，不在验证 lane
    顺手修复。
  - Dependencies: `SP818-T1`, `SP818-T2`, `SP818-T3`, `SP818-T4` 全部提供 fresh focused-test
    output。
  - Covers: `B-001`–`B-014`。
  - Done when: product invariant set 与 Tech mapping/task coverage union 均为
    `{B-001..B-014}`；workflow、format、build、full test 都产生本 session fresh output；
    无 ignored failure、无 weakened assertion、无未解释的 changed file。
  - Verify:
    - `git diff --check`
    - `python3 checks/check_workflow.py --repo .`
    - `python3 checks/check_workflow.py --repo . --spec-dir specs/GH818`
    - `cargo fmt --check`
    - `cargo check --no-default-features`
    - `cargo test --no-default-features`
    - `cargo clippy --no-default-features -- -D warnings`

## PR Handoff

- [ ] `SP818-T6` — implementation PR evidence 与 human handoff — Owner: implementation agent；Done when: 见下；Verify: 见下
  - Owner: implementation agent 起草；maintainer 执行 final review/merge/release。
  - Exclusive files: none；只整理已有 evidence，不改代码或 specs。
  - Dependencies: `SP818-T5`；implementation route gate 仍为 allowed；当前 head 的 diff 与所有
    verification evidence 已刷新。
  - Covers: none（仅交付 issue/spec/test evidence，不实现新的 behavior invariant）。
  - Done when: PR body 使用 `Closes #818` 仅在实现、tests、docs 全部完成时；否则使用
    `Refs #818`。PR 链接 `specs/GH818/product.md`、`tech.md`、`tasks.md`，列出 v069 migration、
    三类 identity、CAS、claim-skip、retry/expiry coalescing、bounded per-row failure recovery、
    WAL/mixed-batch tests、compatibility/rollback 和 fresh command output；声明 security-sensitive
    queue integrity 需要 human review；不声称 agent final approval。
  - Verify:
    - `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`
    - `git status --short`
    - `git diff --name-only origin/main...HEAD`

## Handoff Notes

- Product IDs: `{B-001, B-002, B-003, B-004, B-005, B-006, B-007, B-008, B-009,
  B-010, B-011, B-012, B-013, B-014}`。
- Task coverage union: T1–T5 覆盖同一完整集合；`SP818-T6` 为 evidence-only，故
  `Covers: none`。
- Tech Spec 已固定 migration survivor/Dream profile replay、CompileRules claim/retry/expiry
  coalescing，以及 failure auto-recovery 的逐 row identity contract；实现不得重新打开这些
  选择，除非 maintainer 先修改并批准 Product/Tech contract。
- File ownership 顺序：T2 独占 `state.rs` + `src/db/job/tests.rs`，完成后把 job tests 移交 T3；
  T3 独占 `claim.rs`/enqueue/job tests；T4 只在 T3 后独占 failure lifecycle maintenance/tests
  和 observability files。不存在同时共享 writable file 的获批 lane。
- Coalesced source 的原始/既有截断 `last_error` 必须保留为主证据，marker 在 2000-char 内
  确定性追加；测试和日志证据不得打印原始 error/payload。
- v069 duplicate rows 同样不得覆盖既有 `last_error`：ordinary existing-error 与
  near-2000-char fixtures 必须同时保留 error prefix 和完整 migration marker；marker 单独存储
  只适用于 NULL/empty error，日志禁止输出原 error。migration fixture 还必须用
  `attempt_count < max_attempts` 的 redundant active row 断言真实 attempt evidence 原值保留，
  禁止以伪造 exhausted 代替 `failure_class='permanent'` 的 retry gate。
- 不重新设计 `extraction_tasks`，不恢复 Summary，不加入 process mutex，不新增平行 failure
  ledger。failure-lifecycle recovery 必须在 generic no-active requeue 前排除 retired Summary，
  并以 regression fixture 证明其 terminal audit history 原值保留且不阻塞 ordinary recovery。
- Migration/worker/auth-like execution integrity 属于高风险区域：禁止打印 payload/secrets，
  所有 SQL 参数化，必须 human review。
- 目前仍无 implementation authorization。只有实际 readiness label、`spec_approval`、
  duplicate evidence 和 implement route gate 全部满足后，后续 agent 才能开始 T1。
- final PR review、merge、release 均保留为 human gates；禁止 force push。
