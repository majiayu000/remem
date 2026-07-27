# Task Plan：CurrentTruth 统一读取契约

## Linked Issue

GH-933

PR #939 已于 2026-07-26 合并（merge commit `0ed42e3d`），是 Phase A
baseline，不再是可写 implementation lane。维护者已授权新的 Phase A
hardening follow-up PR 策略并将 issue 标记为 `ready_to_spec`；exact revised
packet 仍需 human `spec_approval`，本计划不表示 Phase A 已 fresh verified，
也不表示 Phase B/C 已完成。

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## 当前证据状态

- PR #939 已合并 Phase A 的 `src/truth*`、`src/lib.rs` export、version-sync
  变更和 18 个 truth tests；这些是 baseline implementation evidence，不是
  本轮 fresh verification，也不能通过继续写其 merged branch 来修复；对应
  source version/release manifest 仍是 staged/unreleased，不是已发布能力。
- Live issue 已有 `ready_to_spec`；本轮 fresh
  `python3 checks/route_gate.py --repo . --route write_spec --issue 933 --state ready_to_spec --json`
  返回 `decision=allowed`。该结果只授权写 spec，不等于
  `ready_to_implement` 或 human `spec_approval`。
- Phase A 待补/待证：cross-project/explicit-branch-scope relation 隔离、
  `branch=None` branch-agnostic 兼容语义、future captured evidence 的
  source/knowledge-time `as_of` filter、memory-row knowledge-time gate、
  user-context-row post-mutation knowledge-time gate、
  routed source-project/foreign-project/dangling/malformed provenance、
  ownership/scope-aware typed memory subject v2、unknown claim status
  diagnostic、operation-backed cross-topic preference conflicts、
  malformed/inconsistent operation-log conflict provenance diagnostics、
  decision-vs-cross-subject-provenance relation split、focused
  Supports/DerivedFrom、SELECT-only/no-write、bounded relation lookup、
  v1-to-v2 intentional golden diff、representative query-plan/performance
  evidence，以及 final-rebase 后绑定 candidate SHA 的完整验证；这些都是下一份
  Phase A hardening follow-up 的 merge 前置项，不得推迟到 Phase B。
- Phase B 的 Context Bundle、worktree/task selector、rollback 与 historical
  explanation，以及 Phase C writer decision/收敛均为后续任务。

## 实现任务

- [ ] `SP933-T1` Owner: coordinator + human maintainer; Dependencies: committed revised `product.md`/`tech.md`/`tasks.md`, existing `ready_to_spec`, human approval of the exact revised packet; Done when: maintainer records `spec_approval`, transitions GH-933 to `ready_to_implement`, and coordinator provides trusted live `origin/main`, approved-spec, duplicate-work, planned-path and `enforcement_sensitive` evidence; duplicate evidence classifies #939 and its retained branch as merged historical baseline, the authorized follow-up is the only writable lane, and the fresh implement route gate returns `decision=allowed`; Verify: `python3 checks/route_gate.py --repo . --route implement --issue 933 --state ready_to_implement <approved-evidence-args> --json`, exact packet commit/hash, human approval evidence and live base SHA.

- [ ] `SP933-T2` Owner: Phase A verification agent; Dependencies: `SP933-T1`; Done when: 在从 fresh `origin/main` 创建的新 follow-up branch 上逐项核对 #939 baseline 的 `CT-001`–`CT-012`、`CT-015`，全量记录 mismatch，确认 adapter 当前 `branch=None` 是 all-branches/branch-agnostic 而 `Some(B)` 是 neutral-plus-exact，核对 canonical writer 的 `(project, scope, memory_type, topic_key)` slot、v019/default owner fallback、routed `source_project`、memory `updated_at_epoch`、captured-event source/inserted fields、raw status allowlists，以及 operation-backed preference conflict edge contract，并记录 v1 golden bytes/hash 作为 v2 intentional-diff baseline；Verify: `cargo test truth -- --nocapture`、writer/schema/selector/source/operation-edge inspection、serialized v1 golden byte hashes、`git diff --check origin/main...HEAD` 和绑定 current follow-up head 的 invariant review notes。
  - Additional done-when: 核对 `src/user_context/claims.rs` 的 edit/suppress/
    unsuppress/delete 都会更新 `updated_at_epoch`，并把 user claim
    created/updated knowledge-time before/equal/after 与 post-cutoff mutation
    列入 mismatch；检查 `memory_operation_log.conflicting_ids` 的 unconstrained
    TEXT 边界、canonical replacement/pairwise membership、owner/type fields，
    明确区分 wholly-unbacked neutral edge 与 claimed-but-malformed/inconsistent
    provenance contextual error。

- [ ] `SP933-T3` Owner: Phase A hardening implementation agent; Dependencies: `SP933-T1`, findings from `SP933-T2`; Done when: writable source files are exactly `src/truth.rs`、`src/truth/adapter.rs`、`src/truth/projection.rs`、`src/truth/tests.rs`、`src/truth/types.rs`；本 owner 实际实现 scoped-ID bounded SQL；升级 `projection_version=2` 与无拼接歧义的 structured `SubjectIdentity/selector`，memory 以 `(Memory,canonical owner_scope,canonical owner_key,Some(normalized scope),memory_type,topic_key-or-singleton)`、user claim 以 `(UserContextClaim,exact owner_scope,exact owner_key,None,claim_type,claim_key)` 分组，owner legacy fallback/partial-field error 严格按 tech spec；Supersedes/普通 Refutes 仅 exact identity 决策，operation-backed canonical `memory_edges.conflicts` 对同 owner/scope/normalized-branch cross-topic preference survivors 执行 narrow post-pass 并将两个 outputs 标为 Contradicted；无 `source_operation_id` 的 unbacked edge decision-neutral，operation-backed cross-owner/scope/branch/type/membership inconsistency contextual-error；scoped cross-subject Supports/DerivedFrom 连接 winner 时保留但 decision-neutral；`branch=None`/`Some(B)` 语义不变；memory source/reference 与 `updated_at_epoch` knowledge time 都不晚于 shared reference epoch；captured evidence 绑定 canonical `source_project`（仅无 routing assertion 的 legacy row fallback `memory.project`），source epoch 与 inserted knowledge epoch 都不晚于同一 reference epoch；late memory/evidence、foreign/missing/ambiguous/malformed 均按规格过滤或 contextual-error；unknown memory/user-claim status contextual-error；Supports/DerivedFrom、SELECT-only 与 v1-to-v2 restricted golden diff 有 regressions；不得修改 writer/schema/context 或整份重录 golden 掩盖 drift，若需新 index/migration/额外 public behavior 必须停止并重新请求 approval；Verify: focused owner/scope typed-subject v2、legacy/partial ownership、branch、same/cross-topic decision/provenance、operation-backed preference conflict、routed source-project/legacy fallback、memory/evidence source+knowledge late-ingest boundaries、unknown/dangling/malformed、bounded SQL、Supports/DerivedFrom、intentional-golden-diff/no-write tests、`cargo test truth -- --nocapture`、`cargo fmt --check`、`cargo check`、`git diff --check`。
  - Additional done-when: user-context claim 仅在 `created_at_epoch` 与
    `updated_at_epoch` 都不晚于 shared reference epoch 时 eligible；post-cutoff
    edit/suppress/unsuppress/delete 的 current row 保守排除/`Unknown`，并有四类
    focused regressions。带 `source_operation_id` 的 conflict edge 必须解析
    `memory_operation_log`，对 missing/wrong operation、malformed JSON、wrong
    element types、duplicate/invalid IDs、result/conflicting endpoint membership
    或 owner/type inconsistency 返回含 edge/operation/endpoints/field 的 contextual
    error；只有无 `source_operation_id` 的 edge 可 decision-neutral。不得修改
    writer/schema 来绕过 read-side validation。

- [ ] `SP933-T4` Owner: read-only Phase A performance verifier; Dependencies: `SP933-T3`; Done when: 在 pre-rebase implementation head 独立验证 relation SQL 不扫描无关 project 全部 `memory_edges`/trusted `graph_edges`，source-project evidence/scoped ID/operation-backed conflict membership 有 bounded argument/row behavior，query plan 使用现有 indexes，v2 bytes 对 T2 v1 hashes 只有批准的 owner/scope identity、cross-topic preference conflict 和 fail-closed intentional diff，并记录 representative corpus、`EXPLAIN QUERY PLAN`、p50/p95、rows examined/returned、memory bound、head SHA 与 migration/index/truth/dependency fingerprints；T4 不拥有源码且不得补实现，发现 gap 必须退回 T3；这是 rebase 前反馈 gate，不是最终 merge evidence；Verify: query-plan/benchmark artifact、restricted golden diff、`cargo test truth -- --nocapture`、`cargo fmt --check`、`cargo check`。
  - Additional done-when: restricted golden/fail-closed diff 明确包含 user-claim
    post-cutoff mutation exclusion与 malformed/inconsistent operation provenance
    contextual errors；验证 wholly-unbacked edge 仍为 decision-neutral，不能把
    新 error contract 扩张到所有外部 edge。

- [ ] `SP933-T5` Owner: Phase A coordinator + read-only T4 verifier; Dependencies: `SP933-T2`, `SP933-T3`, pre-rebase `SP933-T4`; Done when: coordinator rebases on live `origin/main`, resolves without force-push shortcuts，updates `docs/specs/GH933/PRODUCT.md`/`TECH.md` 与 staged/unreleased release metadata，选择高于 final base 的 patch version；在所有 rebase/docs/version edits 完成后，read-only verifier 在 final candidate SHA 从零重跑完整 T4 query-plan/corpus/p50/p95/rows/memory/restricted-golden gate，并重算 migration/index/truth/SQLite-dependency fingerprints；任何后续 commit/rebase（包括只改 version/docs）都使 final T4 artifact 与 full preflight/PR-gate evidence 失效并必须再次重跑；随后 PR 使用 `Refs #933`，不声称 Phase B/C 或 released，不关闭 GH-933，且具备 exact-head full checks、independent review、threads 与 human merge authorization；Verify: final-SHA T4 artifact/fingerprints、current-contract/release-state diff、`cargo fmt --check`、`cargo check`、`cargo test truth -- --nocapture`、`cargo test`、`cargo clippy --all-targets -- -D warnings`、plugin sync/version bump、SpecRail、full preflight、CI 与 current-head `pr_gate`。

- [ ] `SP933-T6` Owner: Phase B spec owner + human maintainer; Dependencies: `SP933-T5`, `SP933-T4`; Done when: Phase A 已有真实运行/性能 evidence，product/tech contract 被更新为可实施的 Phase B closed design，明确 shared render reference epoch、Context Bundle current truth/decisions/conflicts、worktree/task selectors、historical explanation、policy/sensitivity、cache/budget、error-visible behavior、rollout/rollback gate 和 owned files，human 批准 exact spec head，新的 implement route gate 为 allowed; Verify: updated CT mapping、deterministic commands、SpecRail packet check、human `spec_approval`、fresh duplicate evidence 和 allowed route-gate JSON。

- [ ] `SP933-T7` Owner: Phase B context implementation agent; Dependencies: `SP933-T6`; Done when: existing `src/context/*` pipeline 在一次 render 中只调用一次 versioned projection 并共享同一 reference epoch，分别消费 current truths/decisions/conflicts，project/branch/worktree/task scope 均 fail closed，archived evidence 只进入批准的 historical explanation，projection/schema/provenance failure 进入 error-level diagnostics 而不包装为空成功，旧 context path 由批准的 rollout gate 保留且可回滚，Phase B PR 使用 `Refs #933` 且不修改 canonical writers; Verify: focused context load/render/scope/error/rollback/cache-stability/truncation tests、`cargo test context truth`、`cargo fmt --check`、`cargo check` 和 representative SessionStart performance gate。

- [ ] `SP933-T8` Owner: Phase B verification coordinator; Dependencies: `SP933-T7`; Done when: `CT-003`、`CT-008`、`CT-012`、`CT-013`、`CT-015` 的 Phase B rows 有 current-head evidence，old-path rollback、empty、conflict、abstention、suppression、historical、worktree/task isolation、DB/schema/provenance failure 和 performance budgets 全部通过，docs/release guidance 不把 source-only 能力写成已发布，GH-933 保持开放等待 Phase C decision; Verify: `cargo test`、context/eval focused suites、`cargo clippy --all-targets -- -D warnings`、full preflight、independent reviewer、current-head CI/review-thread/merge-state evidence 和 `pr_gate`。

- [ ] `SP933-T9` Owner: Phase C benchmark owner + human architecture reviewer; Dependencies: `SP933-T8`; Done when: benchmark 对 Phase B projection 的 memory quality、conflict/abstention correctness、latency 和 rollback evidence 形成不可变报告，human 明确选择 `converge_writers` 或 `retain_read_projection`；no-go 时记录现有 writers 如何满足 `CT-010`/`CT-014`、剩余限制和长期 owner，go 时批准独立 migration/dual-write/backfill/cutover/rollback spec，禁止直接从 Phase A DTO 推导 schema; Verify: deterministic benchmark command/artifact hashes、architecture decision record、human approval 和 workflow/spec checks。

- [ ] `SP933-T10` Owner: Phase C implementation agent, only when `SP933-T9=converge_writers`; Dependencies: `SP933-T9`, approved Phase C spec, allowed implement route gate; Done when: canonical writer firewall 阻止 generated enrichment 创建/覆盖 canonical Claim，任何批准的 migration/dual-write/backfill/cutover 保留 canonical refs、evidence provenance、scope 和 historical semantics，失败可安全 rollback；未选择 convergence 时此任务标记 `not_applicable` 并引用 T9 human decision，不伪造完成; Verify: approved Phase C focused migration/writer/rollback/security tests、`cargo fmt --check`、`cargo check`、`cargo test`、schema-drift tests 和 benchmark parity。

- [ ] `SP933-T11` Owner: closure coordinator + human maintainer; Dependencies: `SP933-T8`, `SP933-T9`, and `SP933-T10` when applicable; Done when: `CT-001`–`CT-015` 均有 current implementation、test 或批准 no-go evidence，Phase A/B/C 的 PR、release、docs 和 rollback 状态一致，零 actionable acceptance gap、零 unresolved review thread、CI/current-head PR gate green，只有此时最终 implementation PR 才可使用 closing linkage并由 human 决定 merge/release/close GH-933; Verify: spec-vs-implementation review、full repository preflight、closure audit、current-head CI/reviews/merge state、final human authorization 和 allowed `pr_gate`。

## 并行拆分

- `SP933-T1` 是所有 implementation 的串行前置 gate；blocked 时只能继续只读
  review/spec planning，禁止写代码。
- 新 Phase A hardening lane 独占 `src/truth.rs`、`src/truth/adapter.rs`、
  `src/truth/projection.rs`、`src/truth/tests.rs`、`src/truth/types.rs`；
  `SP933-T2`–`SP933-T5`
  默认串行，read-only reviewer 可并行，但不得共享 writable files。
- Current contract `docs/specs/GH933/{PRODUCT,TECH}.md`、version/changelog
  metadata 与 PR body 由 coordinator 在 implementation agent 停止写入后
  单独拥有；spec packet 由 spec owner 独占。不得把 migration、context 或
  writer 路径静默加入 lane。
- Phase B 在 Phase A evidence 与 `SP933-T4` 性能 gate 后串行开始；Phase C
  在 Phase B 验证后开始。不得并行修改 `src/context/*` 与 writers 来提前实现
  未批准 Phase C。
- 如后续启用 subagents，coordinator 必须给出互不重叠的 writable file ownership；
  shared module、spec packet、version files 和 PR body 均保持单 owner。

## 验证

- Fresh implement route gate 返回 `decision=allowed`；本轮 allowed
  `write_spec` gate 不得被解释为 implementation authorization。
- 每个 task 使用自身列出的 focused checks，并把输出绑定到 current head。
- Phase A/Phase B/Phase C 每个 PR 运行 `cargo fmt --check`、`cargo check`、relevant focused tests 与 `git diff --check`。
- Runtime/shared behavior submission 前运行 `cargo test` 和 `cargo clippy --all-targets -- -D warnings`。
- Spec packet 运行 `python3 checks/check_workflow.py --repo . --spec-dir specs/GH933`。
- Distribution changes 运行 plugin version sync、version bump 和 full PR preflight。
- Merge readiness 需要独立 reviewer、fresh CI、current-head review threads、merge state、human authorization 和 read-only `pr_gate`。
- Final T4 performance/query-plan/golden artifact 必须晚于最后一次 rebase/commit，
  与 merge candidate exact SHA 相同；SHA 不同即视为 stale。
- GH-933 只在 `SP933-T11` closure audit 无 gap 后由 human 决定关闭。

## Handoff Notes

- 旧 `.specrail/runtime/rejections/route-gate-933.json` 与 “open PR #939”
  结论是 pre-merge historical evidence，不能代表当前远端事实；#939 已合并。
- Readiness/follow-up strategy 已由维护者授权，live issue 已标记
  `ready_to_spec`。剩余 human gate 是批准本次 committed exact packet，
  随后设置 `ready_to_implement`；本 task packet 本身不是 `spec_approval`、
  final review、merge、release 或 security decision。
- Duplicate gate 必须识别 PR #939 及其 retained remote branch 为 merged
  historical baseline，并绑定新 follow-up branch/PR 的 exact head；不得把
  merged branch 当成可继续修改的原 PR，也不得绕过 duplicate evidence。
- 新 Phase A hardening PR 与后续 Phase B PR 均使用 `Refs #933`，保持 issue
  开放；只有 `SP933-T11` closure audit 后的最终实现才能考虑 closing linkage。
- 所有 checkbox 保持未勾选，因为本 planning turn 没有运行 fresh Rust/CI/
  PR-gate verification。旧 PR body 中的测试描述不能替代 fresh evidence。
- `SP933-T10` 是 conditional task；human 选择
  `retain_read_projection` 时只能以明确 `not_applicable` decision 处理，不能
  假装 writer convergence 已完成。
- 外部 GitHub comment、label、review、merge、release 和 issue closure 都需要
  对应 human gate/authorization；agent 不得自行执行。
