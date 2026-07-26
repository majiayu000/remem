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
| Versioned DTO | `src/truth/types.rs`, `src/truth.rs` | baseline `TRUTH_PROJECTION_VERSION = 1` 且以裸 `subject_key` 表示 subject | hardening 必须因 typed subject identity 显式升级 v2，并更新 public exports/golden；不能无 version boundary 改合法输入结果 |
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
   `branch=Some(B)` 时只接受 branch-neutral 或 exact `B` rows；
   `branch=None` 保持当前 public adapter 的 branch-agnostic 语义，接受该
   project 的全部 branch rows，不解释为 branch-neutral-only。
2. `load_memory_claim_groups` 把 memory 映射为 `ClaimView`。baseline 仅以
   `topic_key` 分组，和 canonical writer 使用的 `(memory_type, topic_key)`
   slot 不一致；缺少 `topic_key` 时以 `memory:<id>` 形成 singleton subject。
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
  edge 影响。follow-up 必须修正并增加 cross-project、
  explicit-branch-scope 外与 cross-subject fixtures，不能让 scope 外 relation
  使 scope 内 claim 被拒绝；另以独立 fixture 锁定 `branch=None` 全分支视图，
  防止调用方把它误解成 neutral-only。
- hardening 将 decision relation 与 provenance relation 分开：
  `Supersedes`/`Refutes` 只有两端属于同一 typed subject 才进入 survivor/conflict
  计算；`Supports`/`DerivedFrom`（及未来明确 allowlist 的 provenance-only kind）
  在两端属于整体 scoped claim set 且一端是 winner 时可跨 typed subject 附加到
  `supporting_relations`，但绝不参与 survivor、evidence tier 或 recency。
- 当前 relation loader 扫描全部 `memory_edges` 和 trusted memory graph
  edges 后在 Rust 中用 `Vec::contains` 过滤。下一份 Phase A hardening
  follow-up 必须把 scope/id 约束下推到 SQL，或实现等价 bounded lookup，
  并在该 follow-up merge-ready 前用 representative corpus、
  `EXPLAIN QUERY PLAN`、p50/p95、rows examined/returned 与 memory bound
  证明不会扫描无关 project 全表；不得推迟到 Phase B。
- 当前 dual-evidence fixture 验证一个 claim 的两个 evidence refs，不验证
  `ClaimRelationKind::Supports` 的 adapter/delivery。Phase A 验证应补一个
  focused Supports relation fixture。
- projection 与 adapter 必须共享一次计算的 reference epoch。对 explicit
  `as_of`，captured evidence 只有 source epoch
  `COALESCE(reference_time_epoch, created_at_epoch)` 与 knowledge epoch
  `inserted_at_epoch` 都 `<= reference_epoch` 时才能进入 trust/output；
  equality 可用。source-before 但 inserted-after 的 late ingestion 必须排除。
  replay 把 `inserted_at_epoch` 后移时允许保守排除（可能少用 evidence），但
  绝不允许回溯抬高历史 truth。before/equal/after 与 late-ingestion fixtures
  必须证明 historical winner 不变。`as_of=None` 也必须共享一次 “now”，避免
  claim/relation/evidence 各自取时造成同次 projection 漂移。
- 每个 captured-event evidence lookup 必须同时 SELECT `project_id` 并 join
  `projects.project_path`，与来源 memory 的 exact `project` path 比较。missing
  project identity、join failure 或 foreign-project event 均沿 `Result` 返回含
  memory canonical ref、event ID、expected/actual project 的 contextual error；
  不能把 foreign verified evidence 当作低 trust 或普通缺失。
- malformed `evidence_event_ids`、malformed user `source_refs_json` 当前会静默
  变为“无 verified evidence/默认 source evidence”，有效 JSON 内不存在的
  captured-event ID 也会被 `continue`。follow-up 必须让三类 provenance
  failure 都沿现有 `Result` path fail closed，并返回包含 claim canonical ref
  与坏 ref/field 的 contextual error；不得把损坏或 dangling provenance 包装成
  较低 trust 的正常成功。future-but-existing event 是按时间排除，不是
  dangling error。若确需扩展已批准范围，必须停止并重新批准 planned paths。
- canonical writer 以 `(project, scope, memory_type, topic_key)` 查找 direct
  upsert slot，因此 hardening v2 引入结构化 `SubjectIdentity { source, kind,
  key }`：memory 映射为 `(Memory, memory_type, topic_key-or-memory:<id>)`，
  user-context claim 映射为
  `(UserContextClaim, claim_type, claim_key)`。`ClaimView`、
  `CurrentTruthView` 与 selector 使用同一 identity，禁止字符串拼接歧义。
  `TRUTH_PROJECTION_VERSION` bump 为 2，`src/truth.rs` 导出新 identity/selector
  类型；v1 没有 release artifact，仍需 changelog/current-contract 明示
  source-level breaking boundary。
- `memory_lifecycle`/`user_claim_lifecycle` 的 total mapping 可继续对未知值
  fail-closed，但实际 adapter 在构建 claim 前必须校验 raw status allowlist；
  unknown memory/user-claim status 返回 table/canonical-ref/raw-value error。
  observations/candidates 在 Phase A 不是 claim source，保持 lifecycle-only
  fail-closed mapping，不能被宣称已有 adapter diagnostic。
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
| CT-001 | v2 `SubjectIdentity`/selector、`src/truth/types.rs`, `src/truth.rs` | same-topic/different-memory-type isolation、singleton/user-claim identity、v1-to-v2 intentional golden diff、`projection_version=2` |
| CT-002 | `src/truth/lifecycle.rs` | 四组 `*_statuses_map_deterministically` tests；assert 四个正交维度 |
| CT-003 | typed subject grouping、decision/provenance relation split、branch scope | explicit/none branch、cross-project/scope decision negatives + scoped cross-subject provenance positive/decision-neutral fixtures；worktree/task 属 Phase B |
| CT-004 | shared reference epoch、source/knowledge evidence epochs、eligibility | existing as-of/expiry + source/inserted before/equal/after + late-ingestion historical-winner regression；hard-delete/status-history limitation仅文档化 |
| CT-005 | `adapter::replacement_relation`, `projection::resolve_group` | `explicit_supersedes_beats_recency`, `user_claim_supersedes_link_resolves_deterministically` |
| CT-006 | project-bound `captured_event_evidence`, trust adapters | existing trust fixtures + same-project positive, foreign/missing-project contextual-error negatives；stored confidence 不在决策输入 |
| CT-007 | `resolve_group` 的 refutes/tie branches | `unresolved_conflict_returns_contradicted_not_a_silent_pick`, `same_tier_same_timestamp_is_contradicted` |
| CT-008 | `eligible`, `abstention`, adapter provenance parsing | baseline abstention + malformed `evidence_event_ids`/`source_refs_json` and dangling captured-event ID contextual-error regressions |
| CT-009 | decision relation set vs provenance-only relation set | same-subject Supersedes/Refutes decision fixtures + same/cross-subject scoped Supports/DerivedFrom retention and decision-neutrality |
| CT-010 | `ClaimSource`, adapter exports, lifecycle-only observation/candidate mapping | code review confirms Phase A claim sources only Memory/UserContextClaim；writer-side firewall 属 Phase C，PR #939 无完成测试 |
| CT-011 | lifecycle mapping + memory/user-claim adapter raw-status validators | known status total mapping + unknown memory/user-claim table/ref/raw contextual-error regressions；observation/candidate limitation documented |
| CT-012 | `eligible` excludes non-Live rows | `archived_and_deleted_rows_never_become_current_truth`; historical explanation consumer 属 Phase B/C |
| CT-013 | future `src/context/*` integration | PR #939 无实现/测试；Phase B 需 context load/render/error/rollback fixtures |
| CT-014 | future writer evaluation in memory/candidate/user-context/graph areas | PR #939 无实现/测试；Phase C 需 benchmark、migration compatibility 和 rollback evidence |
| CT-015 | SELECT-only adapter、v2 version boundary、contextual errors | restricted v1-to-v2 golden diff + SQLite authorizer/`total_changes` no-write + malformed/dangling/foreign-project/unknown-status contextual-error tests |

## 数据流

### Phase A memory projection

```text
TruthQuery(project, branch, as_of, subject)
  -> compute one reference_epoch for the entire projection
  -> SELECT scoped memories ordered by (updated_at_epoch, id)
  -> resolve every captured_event ref + canonical project path
  -> missing/foreign-project ref => contextual error
  -> require source_epoch <= reference_epoch AND inserted_at_epoch <= reference_epoch
  -> bounded SELECT of allowlisted memory_edges + trusted memory graph_edges
  -> ClaimView[] + EvidenceView[] + RelationView[]
  -> structured SubjectIdentity(source, kind, key) grouping
  -> lifecycle/as_of eligibility
  -> same-subject supersedes/refutes -> evidence trust -> recency
  -> attach scoped winner provenance-only relations
  -> CurrentTruthProjection(version=2, truths[])
```

### Phase A user-context projection

```text
(owner_scope, owner_key, as_of)
  -> SELECT exact-owner user_context_claims
  -> source refs + row supersedes links
  -> typed user-claim subject identity
  -> shared resolver
  -> CurrentTruthProjection(version=2, truths[])
```

Phase A 不产生 persistence，不调用 LLM/network，不改变 context。Phase B 才将
projection 作为 context loader 的 typed input；Phase C 才可能改变 writer。

## 兼容性与版本

- #939 baseline 的 `pub mod truth` 对当时 crate 是 additive；本 hardening v2
  对直接使用 v1 truth DTO/selector 的 source caller 是显式迁移边界。现有
  CLI、HTTP、MCP、hook 和 context output 仍不变，因为它们尚未接入 projection。
- v1 baseline 的 scope/as-of/malformed/dangling 修复本可视为 adversarial-input
  compatible corrections；但 same-topic/different-`memory_type` 是合法 canonical
  writer input，typed subject 会改变 observable grouping。因此本 exact packet
  执行上一轮约定的 stop/reapproval 路径：把 `src/truth/types.rs`、
  `src/truth.rs` 纳入 manifest，规定 `projection_version=2` 与结构化
  `SubjectIdentity`/selector，并要求 human 批准 exact spec head 后才能实现。
- v2 intentional golden diff 只允许：projection version、结构化 typed subject
  fields/selector，以及本 packet 明定的 adversarial fail-closed/filtered output。
  `branch=None` all-branches、Some(B) neutral-plus-exact、trust ordering 和其它
  contract-valid resolution 不变。review 必须逐字段对照 v1 fixture，禁止用整份
  golden 重录掩盖额外 drift。
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
  后内存过滤，`memory_ids.contains` 还会带来线性 membership cost。即使
  library-only projection 尚未进入 context hot path，relation 全表扫描也必须
  在下一份 Phase A hardening follow-up 中消除。
- `SP933-T4` 必须在该 Phase A follow-up merge-ready 前记录 representative
  project 的 claim/edge 数量、SQL query plan、p50/p95 latency、
  rows examined/returned 与 memory bound，并证明 relation 查询受 scoped IDs
  约束。Phase B 仍需另行量测 allocated rows、rendered budget，且同一次
  render 只运行一次 projection，但不得承担 Phase A 遗留的 bounded lookup。
- pre-rebase T4 只提供实现反馈。final rebase、current-contract 与 version
  metadata 全部落定后，必须在 merge candidate exact SHA 从零重跑完整 T4，
  并将 SHA、migration/index/truth sources 与 SQLite/Rusqlite dependency
  fingerprints 写入 artifact；其后任何 commit 或 rebase 都使 artifact stale。
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
- provenance malformed/dangling/foreign-project 必须在 Phase A hardening 中
  fail closed；source 与 knowledge epochs 都必须在 trust 前检查，防止未来或
  late-ingested provenance 改写历史查询。

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
- Compatibility: typed subject 触发明确 v2 source-level boundary；v1 未发布但
  source callers 仍需迁移 selector/DTO，intentional golden diff 必须受限；
  Phase B 误把 partial Phase A 当完整 GH-933；旧 schema 缺 column 时查询失败。
- Performance: relation 全表扫描、逐 evidence lookup 和内存 membership 在大库
  上放大；进入 SessionStart hot path 前必须优化并量测。
- Data fidelity: `as_of` 无法重建 hard-delete/status rewrite；captured-event
  replay 会把 `inserted_at_epoch` 后移，历史查询会保守少用 evidence 但不会
  反向抬高；inline `source_refs_json` 没有独立知识时间，最多受 claim row 的
  可见时间约束；malformed/dangling/foreign refs 与 late evidence 必须由本
  hardening fail closed/过滤并有 regression。
- Maintenance: status writer 新增值时 mapper 默认 fail closed，但需要同步测试
  和版本说明；relation allowlist 与 graph contract 必须同步。

## 测试计划

- [ ] Phase A focused: `cargo test truth`
- [ ] Phase A quality: `cargo fmt --check`
- [ ] Phase A lint: `cargo clippy --all-targets -- -D warnings`
- [ ] Phase A regression: `cargo test`
- [ ] 补 typed subject/v2 intentional diff、explicit-branch/branch-none、
      decision-vs-provenance relation、same/foreign project evidence、
      source/knowledge-time/late-ingest boundary、dangling/malformed provenance、
      unknown claim status、Supports/DerivedFrom、SELECT-only/no-write regression。
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
其 `src/truth.rs`、`src/truth/adapter.rs`、`src/truth/projection.rs`、
`src/truth/tests.rs`、`src/truth/types.rs` 修改，并同步回滚该 PR 的
changelog/version metadata；已合并的 #939 baseline、现有 writer、
CLI/HTTP/MCP/hook/context path 无需数据恢复。

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
    "docs/specs/GH933/PRODUCT.md",
    "docs/specs/GH933/TECH.md",
    "npm/remem/package.json",
    "plugins/remem/.codex-plugin/plugin.json",
    "plugins/remem/runtimes/remem-releases.json",
    "server.json",
    "src/truth.rs",
    "src/truth/adapter.rs",
    "src/truth/projection.rs",
    "src/truth/tests.rs",
    "src/truth/types.rs"
  ],
  "spec_refs": [
    "specs/GH933/product.md",
    "specs/GH933/tech.md",
    "docs/specs/GH933/PRODUCT.md",
    "docs/specs/GH933/TECH.md"
  ]
}
-->
