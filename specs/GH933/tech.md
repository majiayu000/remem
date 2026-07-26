# Tech Spec：CurrentTruth 统一读取契约

## Linked Issue

GH-933

PR #939 已于 2026-07-26 合并（merge commit `0ed42e3d`），是使用
`Refs #933` 的 Phase A baseline。维护者已授权新的 Phase A hardening
follow-up 策略与 `ready_to_spec`，但本次修订后的 exact packet 仍需独立
human `spec_approval` 才能进入 implementation。

## Product Spec

[`product.md`](product.md)，行为契约 `CT-001`–`CT-015`。

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Public module boundary | `src/lib.rs`, `src/truth.rs` | 已合并的 PR #939 新增 public `remem_ai::truth` module，导出 DTO、lifecycle mapper、adapter 和 projection API | Phase A 的唯一新调用面；没有 CLI/HTTP/Context Bundle wiring |
| Versioned DTO | `src/truth/types.rs` | `TRUTH_PROJECTION_VERSION = 1`；完整 `CurrentTruthProjection` 携带版本、scope、selector 与 truth rows | 锁定序列化 shape、canonical refs、relations 和 selection reason |
| Lifecycle mapping | `src/truth/lifecycle.rs` | 将 memories、observations、user-context claims、memory candidates 的已知状态映射为 publication/validity/retention/visibility；未知状态 fail closed | 防止把 retention 或 suppression 误当 truth；observations/candidates 在 Phase A 只共享状态语言，不成为 claim source |
| Read adapter | `src/truth/adapter.rs` | 读取 memories、captured events、memory/graph edges 和 user-context claims；生成 `ClaimView`、`EvidenceView`、`RelationView` | 负责 scope、canonical ref、relation direction 和 evidence trust 的边界 |
| Resolution policy | `src/truth/projection.rs` | 按 eligibility、显式 supersedes、refutes、evidence trust、recency 的固定顺序解析；冲突与 abstention 均显式返回 | `CT-003`–`CT-009` 的核心决策路径 |
| Phase A fixtures | `src/truth/tests.rs`, `src/truth/lifecycle.rs` | 使用完整 migrations 的内存 SQLite，现有 18 个 truth tests 覆盖主要 projection 与状态映射 | 验证真实 schema 上的确定性输出，不以 mock DTO 替代 adapter |
| Existing schema contract | `src/migrations/v001_baseline.sql`, `v025_memory_edges.sql`, `v031_graph_edges.sql`, `v049_user_context_claims.sql` 及后续 migrations | 现有 tables/columns/indexes 是 Phase A 的全部数据源；#939 和本 hardening 均不新增 migration | 保持 read-side additive，并暴露 `as_of` 无状态历史表的限制 |
| Current context path | `src/context.rs`, `src/context/query.rs`, `src/context/types.rs`, `src/context/render_inputs.rs`, `src/context/render/*` | 仍按现有 memory/session/workstream pipeline 加载和渲染，不调用 `remem_ai::truth` | Phase B 的候选集成点；Phase A hardening 不改变 context 输出或 fallback |
| Canonical writers | `src/memory/service/save.rs`, `src/memory/store/write.rs`, `src/memory/edge.rs`, `src/memory_candidate/apply.rs`, `src/user_context/claims.rs` | 继续写现有 memories、edges、candidates 和 claims | Phase C 才能评估 writer 收敛；Phase A 不改 writer |
| Distribution | `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `plugins/remem/.codex-plugin/plugin.json`, `plugins/remem/runtimes/remem-releases.json`, `npm/remem/package.json`, `server.json` | 当前 `main@5627a749` 为 `0.6.27`；后续并行 PR 可能先占用下一 patch version | hardening implementation 必须在 rebase 后选择高于 live base 的版本并保持所有 metadata 同步，不能在 spec 中预占固定版本 |

## 设计方案

### Phase A baseline：已合并 PR #939

1. `TruthQuery` 接收 project、optional branch、optional `as_of_epoch` 和
   optional subject selector。memory 查询使用 exact `project` match；
   branch selector 存在时只接受 branch-neutral 或 exact-branch rows。
2. `load_memory_claim_groups` 把 memory 映射为 `ClaimView`。`topic_key`
   相同的 rows 竞争同一 subject；缺少 `topic_key` 时以 `memory:<id>`
   形成 singleton subject。
3. evidence 只保存稳定引用和 trust：
   - captured user/tool event 为 `Verified`；
   - 其它 captured event 为 `ModelGenerated`；
   - `source_trust_class=user_prompt` 为 `Verified`，
     `external_content` 为 `Untrusted`；
   - 其它 memory trust class 不额外抬高 trust，无 ledger evidence 时按
     `ModelGenerated` 竞争；
   - manual user claim 为 `Verified`，third-party statement 为
     `Untrusted`，其它 source kind 为 `ModelGenerated`。
4. relation 只接受明确 allowlist。`memory_edges` 与 trusted、memory-to-memory
   `graph_edges` 可映射 `Supersedes`、`Refutes`、`Supports` 或
   `DerivedFrom`；diagnostic graph edges 不进入 truth。replacement edge 的
   stored direction 会规范化为 `from_ref supersedes to_ref`。
5. lifecycle mapper 对已知状态做 total mapping；unknown string 映射为
   `Candidate + Unknown + Live + Visible`。`valid_to_epoch` 或 expiry 到期后
   动态覆盖为 `Expired`，不回写数据库。
6. projection 先剔除 reference time 尚未创建、未到 valid-from、已过期、
   非 Active、非 Live、Suppressed 或非 Current 的 claim；再应用有效
   supersedes；之后依次处理 refutes、最高 evidence tier 和 recency。
7. 无 surviving claim 返回 `Unknown + InsufficientEvidence`；refutes 或
   同 trust/同 timestamp 的不可破同分返回
   `Contradicted + UnresolvedConflict`，不任选一侧。
8. `CurrentTruthProjection` 只序列化 read DTO，不执行 migration、write、
   LLM、network 或 external process。所有 SQL 继续使用 bound parameters。

### Phase A hardening follow-up：批准范围

- relation 只有在两个 endpoint 都属于同一 query 的 scoped claim set，且属于
  resolver 当前 subject group 时才能参与选择。当前 adapter 以“任一 endpoint
  命中”纳入 relation，resolver 也使用 endpoint OR；现有
  `scope_mismatch_never_leaks_other_projects` 未覆盖 cross-project/cross-subject
  edge 影响。follow-up 必须修正并增加 cross-project、cross-branch 与
  cross-subject fixtures，不能让 scope 外 relation 使 scope 内 claim 被拒绝。
- 当前 relation loader 扫描全部 `memory_edges` 和 trusted memory graph
  edges 后在 Rust 中用 `Vec::contains` 过滤。Phase A library-only 可以先保留，
  但进入 Phase B hot path 前必须把 scope/id 约束下推到 SQL，或用等价 bounded
  lookup，并用 representative corpus 证明延迟和内存上限。
- 当前 dual-evidence fixture 验证一个 claim 的两个 evidence refs，不验证
  `ClaimRelationKind::Supports` 的 adapter/delivery。Phase A 验证应补一个
  focused Supports relation fixture。
- malformed `evidence_event_ids` 和 malformed user `source_refs_json` 当前会
  静默变为“无 verified evidence/默认 source evidence”。follow-up 必须改成
  fail-closed 且可诊断的错误或明确 diagnostic，并用 focused regression 锁定；
  不得把 provenance 损坏包装成较低 trust 的正常成功。该契约可在现有
  `src/truth/adapter.rs` 返回路径内完成；若确需扩展 public DTO 或 module
  boundary，必须停止并重新批准 planned paths，不能静默加入
  `src/truth/types.rs` 或 `src/truth.rs`。
- `as_of` 只能依据现存 row 的 created/validity/relation timestamps；对后来
  hard-delete 或原地 status rewrite 无法完整重建。Phase A 通过规格公开此
  限制，不伪造历史；完整 historical explanation 需要后续持久化契约。

### Phase B：Context Bundle（后续，不属于 Phase A hardening）

1. 使用一次 render 共享的 reference epoch 和 project/branch selector 调用
   versioned projection，避免同次 render 的 sections 观察不同时间。
2. 将 current truths、decisions、conflicts 分别转换为现有 context render
   输入；projection error 必须进入现有 error/diagnostic 路径并以 error level
   可见，不能回传看似成功但缺失 truth 的 bundle。
3. 通过单独批准的 rollout/rollback gate 在旧 context path 与 projection
   path 间切换；本规格不提前声明 config key、CLI flag 或 public API。
4. Phase B 必须增加 worktree/task selector、budget/truncation、cache-stable
   render 和 policy suppression 的产品/技术细化与回归测试。

### Phase C：writer convergence（后续，不属于 Phase A hardening）

1. 先用 benchmark 和 Phase B 运行证据判断是否需要 writer schema 收敛；
   不因 DTO 存在就创建新 canonical tables。
2. 如需 migration，必须另写 migration/compatibility/rollback contract，
   保留 canonical refs、evidence provenance、scope 和 historical semantics。
3. memory、candidate、user-context 与 graph writers 的任何 dual-write、
   backfill 或 cutover 都需要独立任务与 human spec approval；Phase A
   hardening 不包含 writer firewall 完成声明。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| CT-001 | `src/truth/types.rs`, `src/truth.rs` | `golden_projection_shape_is_versioned_and_stable`; `cargo test truth` |
| CT-002 | `src/truth/lifecycle.rs` | 四组 `*_statuses_map_deterministically` tests；assert 四个正交维度 |
| CT-003 | `adapter::load_memory_claim_groups`, `projection::resolve_group`, `TruthQuery` | baseline fixtures + follow-up cross-project/cross-branch/cross-subject relation fixtures；worktree/task 属 Phase B |
| CT-004 | `projection::eligible`, `relation_effective` | `as_of_returns_the_then_current_now_superseded_decision`, `expiry_is_derived_from_validity_window`; hard-delete/status-history limitation仅文档化 |
| CT-005 | `adapter::replacement_relation`, `projection::resolve_group` | `explicit_supersedes_beats_recency`, `user_claim_supersedes_link_resolves_deterministically` |
| CT-006 | `captured_event_evidence`, `trust_class_evidence`, `user_claim_evidence`, `claim_trust_tier` | `verified_evidence_beats_newer_model_generated_claim`, `two_sources_supporting_one_claim_keep_both_evidence_refs`; stored confidence 不在决策输入 |
| CT-007 | `resolve_group` 的 refutes/tie branches | `unresolved_conflict_returns_contradicted_not_a_silent_pick`, `same_tier_same_timestamp_is_contradicted` |
| CT-008 | `eligible`, `abstention`, adapter provenance parsing | baseline abstention fixtures + malformed `evidence_event_ids`/`source_refs_json` fail-closed diagnostic regressions |
| CT-009 | `CurrentTruthView`, relation/evidence adapter | golden shape、dual-evidence、supersedes、conflict fixtures + focused Supports adapter/output fixture |
| CT-010 | `ClaimSource`, adapter exports, lifecycle-only observation/candidate mapping | code review confirms Phase A claim sources only Memory/UserContextClaim；writer-side firewall 属 Phase C，PR #939 无完成测试 |
| CT-011 | `src/truth/lifecycle.rs`, `eligible` | status mapping、unknown fail-closed、expiry tests；`archived_and_deleted_rows_never_become_current_truth`; suppressed path covered by user-claim supersedes fixture |
| CT-012 | `eligible` excludes non-Live rows | `archived_and_deleted_rows_never_become_current_truth`; historical explanation consumer 属 Phase B/C |
| CT-013 | future `src/context/*` integration | PR #939 无实现/测试；Phase B 需 context load/render/error/rollback fixtures |
| CT-014 | future writer evaluation in memory/candidate/user-context/graph areas | PR #939 无实现/测试；Phase C 需 benchmark、migration compatibility 和 rollback evidence |
| CT-015 | SELECT-only `src/truth/adapter.rs`, `TRUTH_PROJECTION_VERSION`, contextual `anyhow` errors | `cargo test truth` + golden shape + SQLite authorizer/`total_changes` no-write regression + malformed provenance diagnostic tests |

## 数据流

### Phase A memory projection

```text
TruthQuery(project, branch, as_of, subject)
  -> SELECT scoped memories ordered by (updated_at_epoch, id)
  -> resolve captured_event refs + memory trust metadata
  -> SELECT allowlisted memory_edges + trusted memory graph_edges
  -> ClaimView[] + EvidenceView[] + RelationView[]
  -> BTreeMap subject grouping
  -> lifecycle/as_of eligibility
  -> supersedes -> refutes -> evidence trust -> recency
  -> CurrentTruthProjection(version=1, truths[])
```

### Phase A user-context projection

```text
(owner_scope, owner_key, as_of)
  -> SELECT exact-owner user_context_claims
  -> source refs + row supersedes links
  -> shared resolver
  -> CurrentTruthProjection(version=1, truths[])
```

Phase A 不产生 persistence，不调用 LLM/network，不改变 context。Phase B 才将
projection 作为 context loader 的 typed input；Phase C 才可能改变 writer。

## 兼容性与版本

- `pub mod truth` 是 additive Rust API；现有 CLI、HTTP、MCP、hook 和 context
  output 不变。
- `CurrentTruthProjection.projection_version=1` 锁定 observable JSON shape；
  字段或 resolution policy 的不兼容变化必须 bump version 并保留明确迁移说明。
- Phase A 读取当前 schema，不新增 migration；旧数据库仍需先由现有 migration
  path 升到当前 schema，projection 自身不得迁移。
- `target_project` fallthrough、global/project ownership convergence、
  worktree/task selector 和 Context Bundle compatibility 均未由 Phase A
  交付，调用方不得据此推断能力。
- hardening follow-up 必须在创建/更新 PR 前 rebase live `origin/main`，选择
  严格高于 live base 的 patch version，并通过 plugin/npm/server/Cargo
  version-sync 与 version-bump checks；本 spec 不预占具体版本号。

## 性能

- scoped memories 使用现有 project/topic/branch indexes；owner claims 使用
  owner-active index。
- 当前 evidence lookup 为逐 claim/逐 event 查询，relation lookup 为全表扫描
  后内存过滤，`memory_ids.contains` 还会带来线性 membership cost。Phase A
  不在 context hot path，风险暂时隔离，但不能直接带入 Phase B。
- Phase B gate 前应记录 representative project 的 claim/edge 数量、SQL
  query plan、p50/p95 latency、allocated rows 和 rendered budget；relation
  查询必须 bounded，且同一次 render 只运行一次 projection。
- 没有数据时返回空/abstention，不应启动 LLM 或 background work。

## 安全

- 所有 SQL 使用 parameters；不拼接 project、branch、owner 或 user content。
- memory 使用 exact project filter，user claim 使用 exact owner pair；
  policy-suppressed rows 不进入 current truth。
- 只有 trusted memory-to-memory graph edges 可进入 projection；
  diagnostic hints 与 stored confidence 不参与 truth。
- Phase A 是 local library surface，无新增 auth boundary；调用者仍可能读取完整
  claim statement，因此未来 HTTP/MCP 暴露必须另做 auth、redaction、sensitivity
  和 policy review，不能直接序列化到网络。
- cross-project relation 必须要求两个 endpoint 都在 scoped set；否则虽不泄露
  scope 外 statement，也可能让 scope 外数据影响 scope 内 truth。
- provenance 解析失败不得抬高 trust；Phase B 前必须可诊断，防止安全性降级被
  误认成正常空 context。

## 备选方案

- 直接重写 canonical schema：拒绝用于 Phase A；风险大且无法先验证 read
  semantics。
- 让每个 context/CLI caller 自行解释状态：拒绝；会产生不一致 truth。
- 用 LLM 或 stored confidence 决胜：拒绝；不可解释且不可稳定重放。
- 仅按 `updated_at_epoch` 取最新：拒绝；会覆盖 supersedes、evidence trust
  和 unresolved conflict。
- Phase A 建 materialized table/cache：暂不采用；当前 projection 可由 canonical
  rows 重建，先通过 benchmark 再决定。

## 风险

- Security: scope 外 relation 影响、未来网络暴露完整 statement、suppression
  绕过和 provenance trust 错分。
- Compatibility: version 1 shape/policy 被调用方固化；Phase B 误把 partial
  Phase A 当完整 GH-933；旧 schema 缺 column 时查询失败。
- Performance: relation 全表扫描、逐 evidence lookup 和内存 membership 在大库
  上放大；进入 SessionStart hot path 前必须优化并量测。
- Data fidelity: `as_of` 无法重建 hard-delete/status rewrite；malformed refs
  当前只有 trust 降级，缺少显式 diagnostic。
- Maintenance: status writer 新增值时 mapper 默认 fail closed，但需要同步测试
  和版本说明；relation allowlist 与 graph contract 必须同步。

## 测试计划

- [ ] Phase A focused: `cargo test truth`
- [ ] Phase A quality: `cargo fmt --check`
- [ ] Phase A lint: `cargo clippy --all-targets -- -D warnings`
- [ ] Phase A regression: `cargo test`
- [ ] 补 cross-project relation、focused Supports relation、malformed provenance
      和 SELECT-only/no-write regression。
- [ ] Version sync:
      `python3 scripts/ci/check_plugin_version_sync.py`
- [ ] Version bump:
      `python3 scripts/ci/check_version_bump.py origin/main HEAD`
- [ ] SpecRail:
      `python3 checks/check_workflow.py --repo . --spec-dir specs/GH933`
- [ ] Phase B：Context Bundle current/decision/conflict、shared `as_of`、error-visible、
      rollback、worktree/task、cache/budget/performance tests。
- [ ] Phase C：writer/migration/benchmark/rollback tests；必须由后续批准规格定义。

## 回滚方案

Phase A hardening 没有 schema 或 data mutation。若 follow-up 有问题，可回滚
其 `src/truth/adapter.rs`、`src/truth/projection.rs`、`src/truth/tests.rs`
修改，并同步回滚该 PR 的 changelog/version metadata；已合并的 #939 baseline、
现有 writer、CLI/HTTP/MCP/hook/context path 无需数据恢复。

Phase B 必须通过单独批准的 rollout gate 保留旧 context path；projection
加载失败时记录 error 并按批准的 rollback policy 切回旧 path，而不是静默输出
缺失 context。Phase C 若引入 migration/dual-write，必须在独立规格中给出
向后兼容读取、停止写入、数据校验与恢复步骤；本 Phase A rollback 不适用于
尚未设计的 Phase C。

## Planned Changes Manifest

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 933,
  "complete": true,
  "paths": [
    "CHANGELOG.md",
    "Cargo.lock",
    "Cargo.toml",
    "npm/remem/package.json",
    "plugins/remem/.codex-plugin/plugin.json",
    "plugins/remem/runtimes/remem-releases.json",
    "server.json",
    "src/truth/adapter.rs",
    "src/truth/projection.rs",
    "src/truth/tests.rs"
  ],
  "spec_refs": [
    "specs/GH933/product.md",
    "specs/GH933/tech.md",
    "docs/specs/GH933/PRODUCT.md",
    "docs/specs/GH933/TECH.md"
  ]
}
-->
