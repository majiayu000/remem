# Task Plan：CurrentTruth 统一读取契约

## Linked Issue

GH-933

PR #939 是原关联 Phase A partial implementation，必须继续使用 `Refs #933`。
本计划不授权新建替代 PR，不表示 Phase A 已 fresh verified，也不表示 Phase B/C
已完成。

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## 当前证据状态

- PR #939 已包含 Phase A 的 `src/truth*`、`src/lib.rs` export、version-sync
  变更和 18 个 truth tests；这些是 existing implementation evidence，不是
  本轮 fresh verification，因此以下任务全部保持未勾选。
- Phase A 待补/待证：cross-project relation 隔离、focused Supports relation、
  malformed provenance、SELECT-only/no-write，以及完整 current-head verification。
- relation bounded query 与 representative performance evidence 必须在 Phase B
  hot-path 接入前完成；不因 Phase A library-only 暂不在 hot path 而静默忽略。
- Phase B 的 Context Bundle、worktree/task selector、rollback 与 historical
  explanation，以及 Phase C writer decision/收敛均为后续任务。

## 实现任务

- [ ] `SP933-T1` Owner: coordinator + human maintainer; Dependencies: accepted `product.md`/`tech.md`/`tasks.md`, readiness label, human `spec_approval`; Done when: coordinator 提供 trusted non-empty base ref、trusted sensitive-path evidence 和 `sensitive_enforcement` evidence，duplicate-work evidence 明确 PR #939 是 GH-933 已授权的原 implementation PR 而不是新重复工作，并在当前 head 重新运行 implement route gate 得到 `decision=allowed`; Verify: `python3 checks/route_gate.py --repo . --route implement --issue 933 --pr 939 --state ready_to_implement <approved-evidence-args> --json` 的 fresh JSON、human gate evidence 和 PR #939 head SHA。

- [ ] `SP933-T2` Owner: Phase A verification agent; Dependencies: `SP933-T1`; Done when: 在原 PR #939 原分支逐项核对 `CT-001`、`CT-002`、`CT-004`–`CT-009`、`CT-011`、`CT-012`、`CT-015`，确认 public DTO/version、lifecycle total mapping、scope/as-of/supersedes/refutes/evidence-trust/recency order、Contradicted/Unknown output 和 canonical/evidence/relation refs 与当前 specs 一致，任何 mismatch 全量列出且不以文档覆盖代码事实; Verify: `cargo test truth`、serialized golden inspection、`git diff --check origin/main...HEAD` 和绑定 current PR head 的 invariant review notes。

- [ ] `SP933-T3` Owner: Phase A implementation agent on original PR #939 branch; Dependencies: `SP933-T1`, findings from `SP933-T2`; Done when: relation 只有两个 endpoint 都属于同一 scoped claim set 时才能参与 resolution，cross-project/cross-branch relation 不泄露也不改变 scope 内 truth，`ClaimRelationKind::Supports` 有真实 adapter/output fixture，malformed `evidence_event_ids` 与 `source_refs_json` 的 fail-closed trust/result 被 focused tests 锁定且产生产品契约要求的可诊断 evidence，projection 经 SQLite authorizer 或等价 `total_changes` regression 证明 SELECT-only，且 fixes 不改 writer/schema/context; Verify: focused cross-scope/Supports/malformed/no-write tests、`cargo test truth`、`cargo fmt --check`、`cargo check`、`git diff --check`。

- [ ] `SP933-T4` Owner: Phase A performance agent; Dependencies: `SP933-T3`; Done when: relation lookup 不再扫描无关 project 的全部 `memory_edges`/trusted `graph_edges`，membership 与 evidence lookup 在 representative corpus 上 bounded，query plan 使用现有或经独立批准新增的 indexes，结果 bytes 与 projection version 1 fixtures 不变，并记录 p50/p95、rows scanned、memory bound；若 Phase A PR 不承载优化，必须建立明确的 Phase B blocking task/PR 且在 Context Bundle wiring 前落地，不能以 later 无 owner 延期; Verify: `EXPLAIN QUERY PLAN` artifact、representative benchmark、`cargo test truth`、full projection golden parity、`cargo fmt --check`、`cargo check`。

- [ ] `SP933-T5` Owner: Phase A coordinator; Dependencies: `SP933-T2`, `SP933-T3`, and either completed `SP933-T4` or a human-approved Phase B blocking handoff; Done when: PR #939 仍为 `Refs #933` 且明确 Phase A partial、不声称 Context Bundle/worktree-task/writer convergence，version `0.6.26` 的 Cargo/plugin/npm/server/release manifests 同步，current-head focused/full checks、preflight、independent review、review threads 和 PR gate evidence 齐全，合并 PR #939 不关闭 GH-933; Verify: `cargo fmt --check`、`cargo check`、`cargo test truth`、`cargo test`、`cargo clippy --all-targets -- -D warnings`、plugin version sync、version bump、SpecRail check、full preflight 和 current-head read-only `pr_gate`。

- [ ] `SP933-T6` Owner: Phase B spec owner + human maintainer; Dependencies: `SP933-T5`, `SP933-T4`; Done when: Phase A 已有真实运行/性能 evidence，product/tech contract 被更新为可实施的 Phase B closed design，明确 shared render reference epoch、Context Bundle current truth/decisions/conflicts、worktree/task selectors、historical explanation、policy/sensitivity、cache/budget、error-visible behavior、rollout/rollback gate 和 owned files，human 批准 exact spec head，新的 implement route gate 为 allowed; Verify: updated CT mapping、deterministic commands、SpecRail packet check、human `spec_approval`、fresh duplicate evidence 和 allowed route-gate JSON。

- [ ] `SP933-T7` Owner: Phase B context implementation agent; Dependencies: `SP933-T6`; Done when: existing `src/context/*` pipeline 在一次 render 中只调用一次 versioned projection 并共享同一 reference epoch，分别消费 current truths/decisions/conflicts，project/branch/worktree/task scope 均 fail closed，archived evidence 只进入批准的 historical explanation，projection/schema/provenance failure 进入 error-level diagnostics 而不包装为空成功，旧 context path 由批准的 rollout gate 保留且可回滚，Phase B PR 使用 `Refs #933` 且不修改 canonical writers; Verify: focused context load/render/scope/error/rollback/cache-stability/truncation tests、`cargo test context truth`、`cargo fmt --check`、`cargo check` 和 representative SessionStart performance gate。

- [ ] `SP933-T8` Owner: Phase B verification coordinator; Dependencies: `SP933-T7`; Done when: `CT-003`、`CT-008`、`CT-012`、`CT-013`、`CT-015` 的 Phase B rows 有 current-head evidence，old-path rollback、empty、conflict、abstention、suppression、historical、worktree/task isolation、DB/schema/provenance failure 和 performance budgets 全部通过，docs/release guidance 不把 source-only 能力写成已发布，GH-933 保持开放等待 Phase C decision; Verify: `cargo test`、context/eval focused suites、`cargo clippy --all-targets -- -D warnings`、full preflight、independent reviewer、current-head CI/review-thread/merge-state evidence 和 `pr_gate`。

- [ ] `SP933-T9` Owner: Phase C benchmark owner + human architecture reviewer; Dependencies: `SP933-T8`; Done when: benchmark 对 Phase B projection 的 memory quality、conflict/abstention correctness、latency 和 rollback evidence 形成不可变报告，human 明确选择 `converge_writers` 或 `retain_read_projection`；no-go 时记录现有 writers 如何满足 `CT-010`/`CT-014`、剩余限制和长期 owner，go 时批准独立 migration/dual-write/backfill/cutover/rollback spec，禁止直接从 Phase A DTO 推导 schema; Verify: deterministic benchmark command/artifact hashes、architecture decision record、human approval 和 workflow/spec checks。

- [ ] `SP933-T10` Owner: Phase C implementation agent, only when `SP933-T9=converge_writers`; Dependencies: `SP933-T9`, approved Phase C spec, allowed implement route gate; Done when: canonical writer firewall 阻止 generated enrichment 创建/覆盖 canonical Claim，任何批准的 migration/dual-write/backfill/cutover 保留 canonical refs、evidence provenance、scope 和 historical semantics，失败可安全 rollback；未选择 convergence 时此任务标记 `not_applicable` 并引用 T9 human decision，不伪造完成; Verify: approved Phase C focused migration/writer/rollback/security tests、`cargo fmt --check`、`cargo check`、`cargo test`、schema-drift tests 和 benchmark parity。

- [ ] `SP933-T11` Owner: closure coordinator + human maintainer; Dependencies: `SP933-T8`, `SP933-T9`, and `SP933-T10` when applicable; Done when: `CT-001`–`CT-015` 均有 current implementation、test 或批准 no-go evidence，Phase A/B/C 的 PR、release、docs 和 rollback 状态一致，零 actionable acceptance gap、零 unresolved review thread、CI/current-head PR gate green，只有此时最终 implementation PR 才可使用 closing linkage并由 human 决定 merge/release/close GH-933; Verify: spec-vs-implementation review、full repository preflight、closure audit、current-head CI/reviews/merge state、final human authorization 和 allowed `pr_gate`。

## 并行拆分

- `SP933-T1` 是所有 implementation 的串行前置 gate；blocked 时只能继续只读
  review/spec planning，禁止写代码。
- 原 PR #939 的 `src/truth/adapter.rs`、`projection.rs`、`types.rs` 与 tests
  共享同一 correctness surface，因此 `SP933-T2`–`SP933-T5` 默认串行；
  read-only reviewer 可并行，但不得与 implementation agent 共享 writable files。
- Phase B 在 Phase A evidence 与 `SP933-T4` 性能 gate 后串行开始；Phase C
  在 Phase B 验证后开始。不得并行修改 `src/context/*` 与 writers 来提前实现
  未批准 Phase C。
- 如后续启用 subagents，coordinator 必须给出互不重叠的 writable file ownership；
  shared module、spec packet、version files 和 PR body 均保持单 owner。

## 验证

- Fresh implement route gate 返回 `decision=allowed`；当前 blocked JSON 不得被解释为 authorization。
- 每个 task 使用自身列出的 focused checks，并把输出绑定到 current head。
- Phase A/Phase B/Phase C 每个 PR 运行 `cargo fmt --check`、`cargo check`、relevant focused tests 与 `git diff --check`。
- Runtime/shared behavior submission 前运行 `cargo test` 和 `cargo clippy --all-targets -- -D warnings`。
- Spec packet 运行 `python3 checks/check_workflow.py --repo . --spec-dir specs/GH933`。
- Distribution changes 运行 plugin version sync、version bump 和 full PR preflight。
- Merge readiness 需要独立 reviewer、fresh CI、current-head review threads、merge state、human authorization 和 read-only `pr_gate`。
- GH-933 只在 `SP933-T11` closure audit 无 gap 后由 human 决定关闭。

## Handoff Notes

- Coordinator rejection evidence：`.specrail/runtime/rejections/route-gate-933.json`。
  该 runtime rejection 是本地 orchestrator evidence，不属于 PR commit；远端
  handoff 以 PR、CI 和 gate artifact 为准。
- 当前 implement route gate 为 `decision=blocked`。明确 rejection：
  `duplicate_work` 检出 open PR #939；trusted default base ref 为空；
  configured sensitive registry 缺 trusted path evidence；
  `sensitive_enforcement` evidence 缺失。
- Human gates 仍包括 `readiness_label` 与 `spec_approval`。本 task packet 不是
  spec approval、final review、merge、release 或 security decision。
- Duplicate gate 必须识别 PR #939 是原关联 implementation lane；不得通过创建
  新 PR、换分支或绕过 evidence 来“解决” duplicate。
- PR #939 的修复只能在原 PR 原分支进行，并持续使用 `Refs #933`；Phase A
  合并后 GH-933 仍开放。Phase B PR 同样是 partial/`Refs #933`。
- 所有 checkbox 保持未勾选，因为本 planning turn 没有运行 fresh Rust/CI/
  PR-gate verification。旧 PR body 中的测试描述不能替代 fresh evidence。
- `SP933-T10` 是 conditional task；human 选择
  `retain_read_projection` 时只能以明确 `not_applicable` decision 处理，不能
  假装 writer convergence 已完成。
- 外部 GitHub comment、label、review、merge、release 和 issue closure 都需要
  对应 human gate/authorization；agent 不得自行执行。
