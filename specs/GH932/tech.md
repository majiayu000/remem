# Context Bundle v1 完整合同 — Tech Spec

## Linked Issue

GH-932

## Product Spec

[`product.md`](product.md)

## Current-Truth Baseline

本设计以 `origin/main` 的 merge commit
`284cdf94406dbbe2583e6ee31f23e2a48af561bf`（PR #938）为基线。PR #938 是
Phase A partial：它建立内部 DTO、deterministic planner、caller-provided executor、基础
budget/audit 与 schema snapshots，并明确把 DB wiring、SessionStart、MCP/REST、doctor 和
benchmark hashes 留给 #932。后续实现必须增量完成这些边界，不能删除已合并测试后声称“重做完成”。

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Public module boundary | `src/context_bundle.rs` | 只导出 `plan(request)` 与 `execute(plan, ExecutorInputs)`；注释明确 DB、SessionStart、MCP/REST、doctor、benchmark 未实现 | 完整实现的统一入口；生产 surface 不应继续暴露 caller-provided candidate 作为主路径 |
| DTOs | `src/context_bundle/domain.rs` | request 有 role/worktree/as-of/risk；plan filters 只有 project/branch/include_superseded/as-of；Bundle 仍是 preferences/lessons/core/workstreams/index/sessions，没有 conflicts/abstentions/evidence_refs/freshness/audit_hash | 必须提升 schema 并完成 issue 原始 Bundle 语义；role/worktree/risk 不得成为 dead fields |
| Planner/hash | `src/context_bundle/planner.rs` | 使用 `ContextLimits::default()`，intent 固定 SessionStart；plan 忽略 role/worktree/risk；`plan_hash` 只在生成时计算 | executor 还需重算验证 hash；role/scope/policy 必须进入 canonical plan |
| Executor/scope | `src/context_bundle/executor.rs` | 只处理调用方传入 candidates；scope 只查 project/branch/trust/superseded；as-of/worktree/role/risk 未执行 | 需要 strict DB adapter、snapshot 与完整 eligibility；公开生产入口不能信任外部 candidate |
| Budget/audit | `src/context_bundle/policy.rs`, `audit.rs` | `chars/4` 只估算 `item.text`；title/heading/ref/fixed render text 不计；validate_plan 只要求非空 hash；audit 无 hash/conflict/abstention | 不满足 strict rendered budget、exact-plan verification 与完整 audit |
| Schema tests | `src/context_bundle/tests/{planner,executor,schema}.rs` | 固定 Phase A plan/bundle JSON 和基础 drop reasons | 作为 schema upgrade、determinism 与 backward-rejection regression 起点 |
| SessionStart loader | `src/context/render_inputs.rs`, `src/context/query.rs`, `src/context/types.rs` | `load_context_render_inputs` 只在 `context` 内可见；各 query error 被记录进 `LoadedContext.errors` 后继续，reference epoch 使用 wall clock | DB adapter 可复用成熟选择，但 Bundle execute 必须把任何 load error 原子提升为 error/blocked，并注入 request as-of |
| SessionStart renderer | `src/context/render.rs`, `src/context/render/{finalize,truncation}.rs`, `src/context/sections/` | 独立 load→select→render→char-limit 路径，最后记录 injection audit；尚未调用 Context Bundle | 新 gate 只能选择 legacy 或 Bundle 单一路径；共享 renderer/identity boundary，避免双选与双注入 |
| Current context audit | `src/context/audit.rs` | injection audit 已有 injected/dropped/abstained、provenance 和 render boundaries | 可映射到 ContextAudit，但必须保证每个 candidate 唯一终态和 hash 一致 |
| MCP | `src/mcp/server.rs`, `src/mcp/server/context_tools.rs`, `src/mcp/types.rs` | tool router 有 recall/timeline/get_observations 等，无 Context Bundle tool | 新建独立 router 文件，避免继续扩大 context_tools；复用 `McpToolError` 稳定错误语义 |
| REST | `src/api/server.rs`, `src/api/handlers.rs`, `src/api/types.rs`, `src/api/handlers/capabilities.rs` | bearer-authenticated localhost `/api/v1/*`，无 context plan/bundle route | 增加 POST surfaces 与 capability；未授权必须在 handler/DB read 前被 middleware 拒绝 |
| Doctor | `src/doctor.rs`, `src/doctor/report.rs`, `src/doctor/types.rs` | 多个静态 checks + JSON schema v3；无 compiler check/structured field | 新 check 只读 schema/config/capability，不读取或输出 payload；增加 root JSON field 时提升 doctor schema |
| Coding bench | `src/eval/coding_bench/{types,condition,runner,artifact,tests}.rs` | remem condition 用 production SessionStart render，但 run artifact 没有 context schema/policy/plan/audit hash 或 rendered budget evidence | 每个 remem-backed run 必须捕获并校验同次 injection evidence |
| Current docs | `docs/specs/GH932/{PRODUCT,TECH}.md`, `docs/specs/README.md` | 明确标记 v1 partial，列出 DB/SessionStart/doctor/benchmark follow-up | 完成实现时更新为完整 current contract；不抹去 Phase A 历史 |

## 设计方案

### 1. Schema/policy upgrade 与 exact hash

- 将当前内部 schema 从 `1` 提升为 `2`，policy 从 `context_bundle_v1` 提升为
  `context_bundle_v2`。产品仍称 Context Bundle v1；数字 `2` 表示已合并 internal contract
  的第二版 schema/policy，不把 Phase A shape 伪装为兼容。
- `ContextPlan` 必须完整包含 normalized project、canonical worktree identity、branch、
  role、risk、as-of、include_superseded、channel order/budgets、estimator version 与
  policy versions。
- `plan_hash` 为 canonical JSON（`plan_hash=""`）的 SHA-256。`execute` 在任何 DB read
  前重算并 constant-time 比较；mismatch 返回 `plan_hash_mismatch`。
- `ContextAudit` 新增 `audit_hash`、snapshot/data-version、rendered budget counts、
  conflicts/abstentions rollup。`audit_hash` 对 `audit_hash=""` 的完整 canonical audit JSON
  计算 SHA-256；entries 先按 `(channel, stable_key, terminal_state)` 稳定排序。
- canonical JSON 只使用 typed structs、ordered vectors/`BTreeMap`；禁止依赖
  `HashMap` iteration order。

### 2. 完整 domain 与 semantic classification

`ContextBundle` schema v2 输出：

```text
schema_version
policy_version
plan_hash
audit_hash
degraded_mode
current_truth[]
decisions[]
constraints[]
failure_lessons[]
workstreams[]
conflicts[]
abstentions[]
evidence_refs[]
freshness
rendered_context
audit
```

Phase A 的 presentation sections 按 canonical type 重新分类：

| Source | Bundle destination |
| --- | --- |
| active current-state / architecture / discovery | `current_truth` |
| decision memory/current-state decision | `decisions` |
| preference、rule、explicit constraint | `constraints` |
| lesson、bugfix/failure lesson | `failure_lessons` |
| active workstream | `workstreams` |
| competing canonical rows without unique temporal/current winner | `conflicts` |
| insufficient provenance/scope/time/derived backing | `abstentions` |
| session summary、source event、projection/backing refs | deduplicated `evidence_refs` |

`memory_index` 与 `recent_sessions` 不再作为语义 top-level section；其 eligible items 按上表
进入语义 section，原始 source/channel 写入 provenance/audit。schema v1 input/plan 明确拒绝，
而不是通过 serde default 混用。

### 3. Typed role/risk/scope policy

- `AgentRole::{Coder,Reviewer,Planner,Researcher}` 与 `RiskClass::{Low,Medium,High}`
  保持 closed enum；plan policy 为每个 role 固定 semantic section priority：

| Role | Stable priority (high → low) |
| --- | --- |
| `coder` | constraints, current_truth, decisions, failure_lessons, workstreams, conflicts |
| `reviewer` | constraints, decisions, conflicts, failure_lessons, current_truth, workstreams |
| `planner` | constraints, current_truth, decisions, workstreams, conflicts, failure_lessons |
| `researcher` | constraints, current_truth, decisions, failure_lessons, conflicts, workstreams |

- role 只决定 priority/section allocation，不改变 owner/project/trust/suppression/temporal
  eligibility。risk policy 只增加 evidence minimum 与 abstention strictness：
  `low` 使用标准 trusted/canonical policy；`medium` 对 projection 要求 canonical_ref +
  evidence_ref；`high` 只允许 trusted canonical selected，其他都有明确 drop/abstention reason。
- worktree 在 planner 前 `canonicalize`，并通过现有 project identity/repo root 逻辑验证；
  plan 保存 canonical repo-relative identity/hash，不把任意绝对路径回显到 MCP/REST/doctor。
- DB query 用 project/owner + branch filter；executor 复查每个 item。branchless/global 行只按
  现有 owner policy 的显式 compatibility rule 进入，并记录 `branchless_owner_compatible`。

### 4. DB-backed executor 与 snapshot

新增 `execute_db(conn, plan)` 作为 production path：

1. 重算/验证 plan、schema/policy/estimator/scope。
2. 在现有 connection 上开启一个 deferred read transaction，并在第一次 read 后固定 SQLite
   snapshot/data version。
3. 通过一个严格 adapter 复用 `load_context_render_inputs`、current-state、lesson、
   workstream、session 与 staleness/temporal helpers，将行映射为 typed candidates。
4. 将 request `as_of_epoch` 传入所有 temporal/staleness/current-state 查询；禁止在 adapter
   内调用 `Utc::now()` 代替。
5. 任一 loader error 都立即取消整个 result，并生成 error-level evidence；不把
   `LoadedContext.errors + partial rows` 发布为 Bundle。
6. 运行统一 eligibility、conflict/abstention、role order、budget/render/audit。
7. 在 transaction 内完成 hash；只在全部验证通过后发布 immutable result。

现有 `execute(plan, ExecutorInputs)` 改为 crate-private fixture seam，或显式命名为
`execute_candidates_for_test`；MCP/REST/SessionStart 不得调用它。此 issue 不新增 migration，
不写 runtime DB。

### 5. As-of 与 conflict/freshness

- 所有 source rows 先检查 `created_at_epoch <= as_of_epoch`；有 valid-from/to、expiry、
  invalidation、supersession 的表按 `[valid_from, valid_to)` 解释。
- current-state 选择使用已有 owner/state-key/temporal predicates；同一 key 在 as-of 时刻
  有唯一 winner 才进入 current truth/decision/constraint。
- 无唯一 winner 的 active canonical rows进入 `ContextConflict`，包含 stable refs 与
  machine reason，不复制不必要 payload。
- 缺少足以证明 as-of eligibility 的 derived item 进入 abstention。legacy canonical rows
  允许 `freshness=unknown`，但 high-risk 不 selected。
- `FreshnessSummary` 固定 reference epoch，并计数 current/stale/superseded/unknown；
  summary 必须能由 item/audit 重算。

### 6. Strict rendered budget

- 新增 `CONTEXT_TOKEN_ESTIMATOR_VERSION = "utf8_upper_bound_v1"`：一个 token budget unit
  取 UTF-8 byte upper bound。它是本地、确定性、保守上界；不声称等于某个 provider tokenizer。
- budget 只约束 `rendered_context`，不约束传输 envelope/audit JSON；REST/MCP 必须分别返回
  `rendered_token_estimate` 和 audit，避免误称整个 JSON 在模型预算内。
- renderer 先把每个完整 item（heading、title、body、citation、separator）渲染为独立 UTF-8
  segment，再按 role priority、section budget、total budget 原子纳入。固定 header/footer
  先计入总预算；若连最小 header 都装不下，正文为空并记录 `budget_below_minimum`.
- 任何 item 只能整体保留或按一个已测试的 safe text boundary 截断；stable key、canonical ref、
  evidence refs 不可截断。最终重新计数并断言 section/total；assert 失败返回
  `render_budget_invariant_failed`，不能发布超限正文。
- audit 记录 pre/post estimate、item boundary、section/total drop reason 与最终 rendered hash。

### 7. SessionStart bridge 与 rollback

- 新增小型 `context::bundle_bridge`：从现有 invocation 构造 schema v2 request，调用
  `plan` + `execute_db`，再把 `rendered_context` 与 Bundle audit 映射到现有 injection gate/
  persistence。
- `src/context/render.rs` 只保留一个早期 path selector：
  `legacy` 或 `bundle_v2`。选择后只执行该路径，禁止先 legacy load 再 bundle load。
- gate 是现有 runtime config 的显式字段，初始默认 `legacy`；doctor 同时报告 configured/effective
  path。兼容 fixture 通过后才可另行批准默认切换。
- parity 比较可见 semantic items、order、drop reasons、budget 和 empty/error behavior；
  允许新 Bundle header/audit envelope 的 versioned 差异，不允许遗漏当前可见 canonical data。
- rollback 只把 selector 改回 `legacy`，不删 schema/data/audit；严格 injection gate 继续阻止
  同 session 双重注入。

### 8. MCP/REST surfaces

- MCP 新建独立 router：
  - `context_plan`：只调用 planner，返回 plan JSON；
  - `context_bundle`：打开只读 current-schema DB，调用 `execute_db`。
- REST 新增：
  - `POST /api/v1/context/plan`
  - `POST /api/v1/context/bundle`
- 两个 transport 复用 domain DTO/validation 与 error code mapping。REST response envelope 增加
  `experimental: true`；capabilities 暴露 schema/policy/tool/endpoint 名称。
- HTTP mapping：invalid schema/request/hash=`400`，unauthorized=`401`，scope/safety
  blocked=`409`，DB/internal invariant=`500`，cancel/deadline=`503`。MCP 使用同名稳定
  `code` 字段。
- REST auth middleware 在 handler 前运行；测试以一个“DB read sentinel”证明未授权请求没有
  打开/查询 payload。

### 9. Doctor 与 benchmark evidence

- 新增 `doctor::context_bundle` check，报告 schema、policy、estimator、DB schema readiness、
  configured/effective SessionStart path、MCP/REST capability 和 closed degraded reason。
  使用 synthetic empty request/plan 或静态 capability，禁止加载 memory rows。
- `doctor --json` 增加 typed `context_compiler` root object并提升
  `REPORT_SCHEMA_VERSION`；text/JSON 使用同一 snapshot，不依赖颜色。
- coding-bench 的每个 remem-backed `RunReport` 增加
  `ContextBundleEvidence { schema_version, policy_version, estimator_version, plan_hash,
  audit_hash, degraded_mode, rendered_token_estimate, token_budget, rendered_sha256,
  source_head_sha, fixture_sha256 }`。
- remem condition 必须从实际 production SessionStart/Bundle bridge 捕获 evidence；validator
  重算格式/hash/预算并绑定同 run head + fixture。control conditions 明确 `null/not_applicable`。
  不能从固定字符串或另一次 synthetic plan 填充。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001` | domain schema、serde、transport DTO | schema v1 rejection、unknown enum、schema v2 snapshots |
| `B-002` | planner canonicalization/hash | repeated plan、env/clock/order independence、request mutation matrix |
| `B-003` | DB snapshot executor、stable ordering、audit hash | same snapshot byte equality、reverse insertion、hash mutation tests |
| `B-004` | planner/executor boundary | plan-only DB read sentinel；tampered plan rejected before DB read |
| `B-005` | schema v2 semantic sections | full/empty JSON snapshots；legacy presentation-source classification matrix |
| `B-006` | candidate mapper/provenance policy | canonical/generated/graph-derived positive/negative matrix |
| `B-007` | audit terminal-state ledger | selected/dropped/abstained/conflict exactly-once property tests |
| `B-008` | conflict resolver、freshness summary | unique winner、two-active conflict、unknown provenance、summary recomputation |
| `B-009` | worktree/project normalization + executor recheck | same repo、symlink、other repo、missing/permission-denied worktree fixtures |
| `B-010` | DB branch predicate + executor | current/main/other/branchless/global closed matrix |
| `B-011` | role policy table | all role variants exact priority/plan-hash delta；scope safety invariant |
| `B-012` | as-of query/eligibility | before-created、valid interval boundary、expired/invalidated/current clock independence |
| `B-013` | supersession policy | default exclusion、explicit history inclusion、never-current assertions |
| `B-014` | risk policy table | all risk variants；high-risk derived/quarantined/unknown provenance rejected |
| `B-015` | segmented renderer + estimator | multibyte/title/ref/header/footer all counted；property budget never exceeded |
| `B-016` | renderer truncation/audit | section/total/tiny-budget、UTF-8/item-boundary、audit/body parity |
| `B-017` | degraded/blocked state machine | enrichment unavailable、schema/DB/scope/hash failures、error-level capture |
| `B-018` | execute atomic publish | empty success、each loader failure、render failure、cancel/deadline no-partial |
| `B-019` | SQLite read transaction | concurrent writer barrier proves one snapshot；parallel hash isolation |
| `B-020` | deny-network/auth boundary | network sentinel；REST unauthorized pre-DB-read；MCP local-only |
| `B-021` | SessionStart bundle bridge | legacy/bundle semantic parity、single load/single injection、audit persistence |
| `B-022` | runtime config selector | default legacy、explicit bundle、invalid config fail-visible、rollback fixture |
| `B-023` | MCP/REST handlers | cross-transport golden equality、schema/error/auth/backward-compat tests |
| `B-024` | doctor check/report | text/JSON states、no query/title/content/secret leakage sentinel |
| `B-025` | coding-bench run evidence/validator | required remem fields、tampered hash/budget/head/fixture negative fixtures |
| `B-026` | planner/executor/surfaces | deny all network/LLM hooks；no downloaded files/jobs |
| `B-027` | compatibility/presentation | gate-off baseline parity、experimental marker、ASCII/JSON state labels |

## 数据流

```text
ContextRequest
  -> normalize project/worktree/branch + closed role/risk/as-of validation
  -> deterministic plan(policy v2)
  -> canonical plan JSON -> plan_hash
  -> execute_db:
       verify schema/policy/hash before payload read
       begin one read transaction / freeze data version
       strict DB adapter (memories/current state/lessons/workstreams/sessions)
       provenance + temporal + owner/project/branch/worktree eligibility
       semantic classification
       conflict / abstention resolution
       stable role ordering
       segmented render + section/total budget
       exactly-once audit -> audit_hash
       commit read transaction / publish immutable Bundle
  -> one of:
       SessionStart injection gate
       MCP context_bundle
       REST POST /api/v1/context/bundle
       coding-bench evidence

any validation/query/render/cancel failure
  -> discard temporary candidates/body/audit
  -> error-level diagnostic + stable transport outcome
  -> no partial Bundle, no DB write, no silent legacy fallback
```

## 备选方案

- **继续让调用方构造 `ExecutorInputs`**：拒绝。调用方可伪造 provenance/scope，且无法证明
  同一 DB snapshot。
- **SessionStart 与 Bundle 各保留一套 selection**：拒绝。parity 会再次漂移，不能满足单一
  context compiler。
- **沿用 `chars/4` 只数 body**：拒绝。标题、引用和固定文本可让实际 rendered output 超预算。
- **直接使用 provider tokenizer**：拒绝。会引入模型耦合、网络/依赖和跨 host 不确定性；
  使用 versioned conservative local estimator。
- **DB 部分失败时返回剩余 sections**：拒绝。用户会把 incomplete context 当完整事实；
  Bundle execute 必须原子失败。
- **未知 role/risk/version alias 到默认值**：拒绝。API 边界不存在 alias，必须显式错误。
- **新 SessionStart 默认立即替换旧路径**：拒绝。先用显式 gate 证明 parity 与 rollback。
- **doctor 通过执行真实 query 检查**：拒绝。会泄漏 payload、改变性能并把“无数据”误作故障。
- **只在 benchmark report 记录 plan hash 字符串**：拒绝。必须绑定同次 audit、render、head、
  fixture 和预算，并用负例验证。

## 风险

- Security: 新 REST/MCP surface 扩大 memory 读取面；继续使用 bearer/local boundary，所有 scope
  参数化查询，未授权 pre-read 拒绝，不在 doctor/log 输出 payload。
- Data integrity: as-of/current-state 映射错误可能返回过期或未来 truth；以闭合 temporal matrix、
  conflict abstention 和 high-risk fail-closed 防护。
- Compatibility: schema v2 和 SessionStart renderer 有可见差异；明确 version、default legacy
  gate、parity fixture 与 rollback。
- Performance: one snapshot 加完整 audit/hash 有成本；候选和 rendered budget 都有硬上限，
  benchmark/doctor 记录时延但不以静默跳过换性能。
- Maintenance: `src/context/render.rs` 已接近 800 行；只加 path selector，新增逻辑放在
  `context_bundle/` 与小型 bridge/handler/check 文件。
- Privacy: evidence refs、worktree 和 doctor diagnostics 可能泄漏路径/内容；外部 DTO 使用 stable
  refs/identity，不回显任意绝对路径，doctor 只报状态/hash/version。

## 测试计划

- [ ] Unit: schema/hash/role/risk/scope/as-of/supersession/provenance/conflict/audit/budget。
- [ ] DB integration: real migrations + one read transaction + concurrent writer + every loader
      failure/cancellation fixture。
- [ ] SessionStart: legacy/bundle parity、single injection、gate/rollback、empty/error/tiny budget。
- [ ] MCP/REST: cross-transport golden、tool/schema metadata、auth pre-read、stable error codes、
      capabilities。
- [ ] Doctor: text/JSON states、schema bump、payload/secret leak sentinel。
- [ ] Coding bench: same-run evidence capture、hash/head/fixture/budget tamper negatives。
- [ ] Offline: deny network/LLM/download/jobs for planner/executor/SessionStart/doctor/benchmark。
- [ ] Required gates: focused tests, `cargo fmt --check`, `cargo check`, `cargo test`,
      `cargo clippy --all-targets -- -D warnings`, plugin/version sync and full PR preflight。

## 回滚方案

若 Bundle DB path 或 renderer 在发布前/后出现回归：

1. 将 SessionStart selector 显式切回 `legacy`，保留 schema v2 代码、transport experimental
   marker、audit evidence 与 DB 数据；
2. 若 MCP/REST surface 有 contract bug，通过 capability effective=false 关闭执行但保留稳定
   unsupported/blocked response，不删除旧 endpoint 或静默返回 legacy search；
3. 回退实现 commit 时同步回退所有 version metadata/changelog/docs，重新运行 full test/preflight；
4. 不删除 memory、不降级数据库 schema、不关闭 error log、不把 partial Bundle 当 empty；
5. 任何默认切换或 release 仍需 maintainer 的 review/merge/release gate。

## Planned Changes Manifest

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 932,
  "complete": true,
  "paths": [
    "specs/GH932/product.md",
    "specs/GH932/tech.md",
    "specs/GH932/tasks.md",
    "docs/specs/GH932/PRODUCT.md",
    "docs/specs/GH932/TECH.md",
    "docs/specs/README.md",
    "README.md",
    "docs/ARCHITECTURE.md",
    "CHANGELOG.md",
    "src/context_bundle.rs",
    "src/context_bundle/domain.rs",
    "src/context_bundle/planner.rs",
    "src/context_bundle/policy.rs",
    "src/context_bundle/hash.rs",
    "src/context_bundle/executor.rs",
    "src/context_bundle/db_executor.rs",
    "src/context_bundle/render.rs",
    "src/context_bundle/audit.rs",
    "src/context_bundle/tests/mod.rs",
    "src/context_bundle/tests/planner.rs",
    "src/context_bundle/tests/executor.rs",
    "src/context_bundle/tests/db_executor.rs",
    "src/context_bundle/tests/render.rs",
    "src/context_bundle/tests/schema.rs",
    "src/context.rs",
    "src/context/bundle_bridge.rs",
    "src/context/render.rs",
    "src/context/render_inputs.rs",
    "src/context/query.rs",
    "src/context/types.rs",
    "src/context/audit.rs",
    "src/context/tests/mod.rs",
    "src/context/tests/bundle_bridge.rs",
    "src/context/tests/gate_pipeline.rs",
    "src/context/tests/render.rs",
    "src/context/tests/truncation.rs",
    "src/runtime_config.rs",
    "src/mcp/server.rs",
    "src/mcp/server/context_bundle_tools.rs",
    "src/mcp/server/tests.rs",
    "src/mcp/server/tests/tool_metadata.rs",
    "src/mcp/server/tests/context_bundle.rs",
    "src/mcp/types.rs",
    "src/api/server.rs",
    "src/api/handlers.rs",
    "src/api/handlers/context_bundle.rs",
    "src/api/handlers/capabilities.rs",
    "src/api/types.rs",
    "src/api/tests.rs",
    "src/api/tests/context_bundle.rs",
    "tests/api_public.rs",
    "src/doctor.rs",
    "src/doctor/context_bundle.rs",
    "src/doctor/report.rs",
    "src/doctor/types.rs",
    "src/doctor/tests.rs",
    "src/doctor/tests/context_bundle.rs",
    "src/eval/coding_bench/artifact.rs",
    "src/eval/coding_bench/condition.rs",
    "src/eval/coding_bench/runner.rs",
    "src/eval/coding_bench/types.rs",
    "src/eval/coding_bench/tests.rs",
    "eval/coding-bench/README.md",
    "Cargo.toml",
    "Cargo.lock",
    "plugins/remem/.codex-plugin/plugin.json",
    "plugins/remem/runtimes/remem-releases.json",
    "npm/remem/package.json",
    "server.json"
  ],
  "spec_refs": [
    "specs/GH932/product.md",
    "specs/GH932/tech.md",
    "specs/GH932/tasks.md",
    "docs/specs/GH932/PRODUCT.md",
    "docs/specs/GH932/TECH.md"
  ]
}
-->

## Authorization Boundary

本文件与 `tasks.md` 是 planning evidence，不是 `spec_approval`。只有维护者批准 product/tech
exact diff、issue 获得有效 readiness state、`ready_to_implement` route gate 返回 `allowed`
后，implementation agent 才能修改 manifest 中的 production paths。最终 review、CI、
merge、security 和 release 人工门保持不变。
