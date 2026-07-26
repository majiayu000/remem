# Tech Spec

## Linked Issue

GH-934

## Product Spec

[`product.md`](product.md)

## 基线与实现状态

本 spec 以原 PR #940 head `7c327b7b2df7db3ce86c6b934da49d25f2c497ef` 的 diff 为
Phase A 部分实现证据。该 PR 当前以 `codex/issue932-context-bundle-v1` 为 base，GitHub
`mergeStateStatus=DIRTY`；实施前必须刷新 remote truth，在原 PR/原分支处理 base 与冲突，不能创建
替代 PR。Phase A 已实现 plan compilation/debug，但没有 DB-backed execution、公开 Context Bundle
adapter、per-intent eval、ablation 或 default gate。

`docs/specs/GH934/{PRODUCT,TECH}.md` 是当前 remem contract，明确把 execution wiring、golden
fixtures 与 ablation 留作 follow-up。本 packet 细化剩余 issue 验收；实现必须同步更新 current
contract，不能把历史 Phase A 文本当成完整完成证明。

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Router DTO / planner | `src/retrieval_router.rs`, `src/retrieval_router/domain.rs`, `src/retrieval_router/intent.rs`, `src/retrieval_router/planner.rs`, `src/retrieval_router/tests.rs` | Phase A 定义 15 个 channel、六个 intent、deterministic mapping、high-risk adjustment 与 stable hash；只编译 plan | 保留唯一 intent/policy vocabulary；禁止在 adapter/eval 复制映射 |
| Context Bundle DTO/audit | `src/context_bundle/domain.rs`, `src/context_bundle/audit.rs` | `ContextAudit` 只记录 GH-932 `ContextPlan.plan_hash` 和六个 SessionStart section 的结果 | 需要加入 retrieval policy/intent/plan/channel execution audit |
| Context Bundle execution | `src/context_bundle/executor.rs`, `src/context_bundle/tests/executor.rs` | 只执行 caller-provided candidates；无 DB access，按 section/filter/budget 产出 bundle | 复用 scope、budget、degraded/blocked 与 audit 逻辑，新增 router entrypoint |
| Production context loaders | `src/context/query.rs`, `src/context/implicit_query.rs`, `src/context/hybrid_context.rs`, `src/context/types.rs` | SessionStart 独立加载 memory、lesson、summary、workstream，并使用静态 relevance/rerank path | 提供已有 trusted readers；不允许 router 重写 eligibility/suppression/staleness |
| Curated retrieval / fusion | `src/retrieval/search/memory/runner.rs`, `src/retrieval/search/memory/text.rs`, `src/retrieval/search/memory/weights.rs`, `src/retrieval/search/memory/explain.rs` | 静态 channel weights + weighted RRF，source-anchor/confidence gate 后统一 rerank | Router execution 必须把 channel plan 编译成可执行 limits/weights/caps，并保留现有 safety gates |
| Entity/graph/temporal/vector | `src/retrieval/entity/`, `src/retrieval/graph/`, `src/retrieval/temporal/`, `src/retrieval/vector_candidates.rs` | 已有独立检索能力 | 作为 adapter 被 plan 调度；不得新增平行 SQL/索引 |
| Rerank | `src/retrieval/rerank/stage.rs`, `src/retrieval/rerank/types.rs`, `src/retrieval/rerank/tests.rs` | GH-851 统一 post-eligibility rerank，off/failure 保留 baseline order | Router 仅传 participation/N/k/fallback/canonical requirement |
| Enrichment | GH-933 原 PR #939 及合并后的 canonical projection implementation；`src/memory/retrieval_enrichment.rs` 与 tests 是当前已存在的 enrichment truth | 当前 #940 worktree 尚未含 GH-933 execution projection；Issue 要求 generated 与 canonical 分离 | 实现前先以 post-merge code truth 确认 exact anchor，复用其 source binding；不得创建第二套 projection |
| Service boundary | `src/memory/service/types.rs`, `src/memory/service/search.rs` | MCP/REST/CLI search 共享 `SearchRequest`，默认 static fusion | 新 Context Bundle service 应复用共享 DB access，不把 router 逻辑散落到 adapters |
| MCP | `src/mcp/types.rs`, `src/mcp/server/context_tools.rs`, `src/mcp/server/tests.rs` | 有 user recall、timeline、observation details；没有 versioned Context Bundle tool | 新 `context_bundle` 参数直接承载 typed request/intent，稳定错误且无隐藏 fallback |
| REST | `src/api/server.rs`, `src/api/types.rs`, `src/api/handlers.rs`, `src/api/handlers/capabilities.rs`, `src/api/tests.rs` | 有 GET search 与 POST user recall；没有 generic Context Bundle endpoint | 新 `POST /api/v1/context` 返回 versioned bundle/audit，capability map 显式声明 |
| CLI debug | `src/cli/context_types.rs`, `src/cli/actions/context_plan.rs` | Phase A `remem context-plan` 只编译并输出 plan，不开 DB | 保持纯 debug；可增加 execution/audit smoke，但不能让 debug 改数据 |
| Golden/eval gates | `src/eval/golden/`, `src/eval/gates.rs`, `src/eval/gates/tests.rs`, `eval/gates/{baseline,thresholds}.json` | 通用 deterministic retrieval golden 与门禁，没有 intent/router ablation 维度 | 增加独立 router suite、指标与 fail-closed default decision |
| Benchmark artifacts | `src/eval/bench_artifact/types.rs`, `src/eval/bench_artifact/verify.rs`, `src/eval/coding_bench/{types,artifact,condition,run_plan,runner}.rs`, `eval/public/schemas/` | artifact 可验证 environment/current-memory evidence，但无 retrieval plan hash/policy/intent | 写入并验证 router audit、dataset/implementation fingerprints 与 memory-hurt |
| Status/docs/version | `src/cli/actions/query/status.rs`, `src/cli/actions/query/status/types.rs`, `README.md`, `docs/ARCHITECTURE.md`, `CHANGELOG.md` 与五个 version-sync surface | 不显示 router rollout/default gate 状态 | active mode/policy/report fingerprint 与 rollback 必须可见且同步 |

## 设计方案

### 1. 保留 Phase A 为唯一 plan compiler

`retrieval_router::planner::plan(&ContextRequest, Option<ContextIntent>)` 继续是唯一 intent
resolution 与 policy compiler。execution、MCP、REST、eval 都只能调用该函数，不能维护第二份
keyword table、channel matrix 或 high-risk adjustment。

Phase A 的 schema/policy 常量与 `plan_hash` 算法保持兼容。若需要改变 serialized shape 或 mapping，
必须提升 schema/policy version、保留旧 artifact verifier 的明确拒绝/兼容结论，并更新所有 golden
fixtures；不能在同一 version 下静默改变 hash 输入。

### 2. DB-backed channel executor

新增 `src/retrieval_router/executor.rs` 与
`src/retrieval_router/executor/{channels,fusion,audit,tests}.rs`：

```text
ContextRequest + explicit_intent
  -> planner::plan
  -> validate plan/schema/hash
  -> execute enabled channels in RetrievalChannel::ORDERED
  -> normalize (canonical_ref, projection_ref, trust, validity, score)
  -> dedupe canonical/projection identity
  -> apply per-channel limit/weight/contribution cap
  -> weighted fusion
  -> source-anchor / suppression / trust / freshness gates
  -> optional GH-851 rerank under plan policy
  -> abstention
  -> ContextBundle + ContextAudit
```

`ChannelExecutor` trait 只抽象测试输入与现有 reader 调用，不抽象成 `Any` 或 stringly typed public API。
production implementation 持有 caller 已打开的 `rusqlite::Connection`，复用现有
`retrieval/search`、context readers、workstream、summary、git trace 与 benchmark evidence helpers。
所有 SQL 保持在现有 parameterized reader 内；router 不拼接 SQL。

每个 channel 返回 typed `RetrievedCandidate`：

- `retrieval_channel`
- canonical stable key / canonical reference
- optional projection reference
- bundle `ChannelKind`
- source kind、trust、validity、project、branch
- raw channel score/rank
- bounded evidence refs

candidate 正文只在通过现有 eligibility/suppression/source-anchor gate 后进入 bundle。channel error
转换为 `ChannelExecutionAudit`，根据 `ChannelDegradation` 执行 `skip_channel` 或 `fail_closed`；
disabled channel 不调用 adapter。

### 3. Channel mapping 与现有能力边界

执行映射固定并 test-lock：

- `canonical_fts` / `canonical_vector` / `entity_graph` / `graph_expansion` / `temporal`：
  复用 `src/retrieval/search/memory/text.rs` 及对应 retrieval modules；
- `workstreams`：复用 `workstream::query_active_workstreams`；
- `session_outcomes`：复用 `context::query_recent_summaries` 的 safe eligibility path；
- `decisions` / `preferences` / `constraints` / `failure_lessons`：复用当前 typed memory/lesson readers
  与 owner/suppression/staleness policy；
- `superseded_history`：仅在 plan validity/freshness 允许时读取；
- `git_evidence`：复用 `src/git_trace.rs` 的 project/session-bound evidence；
- `benchmark_evidence`：只读取经过 verifier 的 current benchmark metadata，不把任意报告文本注入；
- `generated_enrichment`：只复用 GH-933 合并后的 source-bound projection reader。

映射到 Bundle section 也固定：preferences→Preferences、failure_lessons→Lessons、
workstreams→Workstreams、session_outcomes→Sessions；current trusted decision/constraint/evidence 可进入
Core，其余合法 memory/history 进入 MemoryIndex。一个 canonical stable key 最终只进入一个最高优先
section；audit 保留它被其它 channel 命中的贡献，但 contribution cap 以 canonical identity 计数。

### 4. Fusion、projection 与 corrupted enrichment 防线

新增 fusion input identity `(canonical_ref, projection_ref, retrieval_channel)`。canonical FTS/vector
可分别提供真实独立检索证据；generated projection 必须保留 canonical binding，且所有由同一
projection 文本衍生的 FTS/vector 命中合并为一个 `generated_enrichment` contribution。缺失或无法
验证 canonical binding 时，该 projection 不进入 fusion。

fusion 顺序：

1. channel 内按稳定 `(score desc, canonical stable key asc)` 排序；
2. 截断 candidate limit；
3. 对相同 canonical/projection pair 去重；
4. 应用 channel weight 与 `max_contribution`；
5. 跨 channel weighted RRF；
6. 现有 source-anchor/confidence/trust/freshness gate；
7. plan 允许时调用 GH-851 rerank；
8. 执行 canonical Top 1 与 abstention。

corrupted-enrichment fixtures 至少覆盖错误 canonical ID、跨 project binding、无 source、重复
projection、同 projection 双命中 FTS/vector、恶意高分正文、timeout 与 provider error。所有 fixture
断言 high-risk Top 1 不由 generated-only 证据产生，且 failure reason 可审计。

### 5. Context Bundle / Audit 统一

扩展 `ContextAudit`，增加：

- `retrieval_policy_version`
- `retrieval_intent`
- `retrieval_plan_hash`
- `retrieval_mode`（`static_fusion` / `intent_router`）
- `channel_executions[]`
- `abstention_reason`
- aggregate retrieval latency

`ChannelExecutionAudit` 记录 channel、enabled、candidate/selected/contribution 数、degradation、
reason codes、timeout/error class 与 latency，不包含 memory 正文或 secret。新增
`context_bundle::executor::execute_retrieval_plan`，复用现有 scope/budget/audit helpers；旧
`execute(&ContextPlan, ...)` 保持 GH-932/SessionStart compatibility。

router bundle 的 top-level `plan_hash`、`ContextAudit.retrieval_plan_hash` 与实际 executor 接收的
`RetrievalPlan.plan_hash` 必须相同。bundle-plan 与 retrieval-plan 分离的旧字段不能冒充该 hash；
schema tests pin exact JSON。

### 6. MCP / REST 显式 intent

在 `src/mcp/types.rs` 定义 versioned `ContextBundleParams`，字段与 `ContextRequest` 一一映射，另有
optional explicit intent。在 `src/mcp/server/context_tools.rs` 增加 `context_bundle` tool：

- adapter 只做 serde/validation、project/cwd normalization、DB open 与 error mapping；
- 显式 intent 原样交给 planner；
- 返回完整 `ContextBundle` JSON；
- invalid enum/schema/scope/budget 返回 `invalid_request`，DB/channel fail-closed 返回稳定 tool error；
- 默认日志仅记录 stable metadata/hash/reason，不记录 bundle 正文。

在 REST 增加 `POST /api/v1/context`，request/response 使用相同 DTO，错误为稳定 JSON 4xx/5xx。
`/api/v1/capabilities` 声明 exact endpoint/schema/policy version。现有 GET search、MCP search 与 CLI
search 不改变默认 shape；若以后内部复用 router，只能调用同一 service/executor。

### 7. Rollout 与 default gate

`RetrievalRouterMode` 为 versioned enum：

- `static_fusion`：现有默认；
- `explicit_only`：只有显式合法 intent 的 Context Bundle surface 执行 router；
- `intent_router`：default gate 通过后的默认，缺 intent 用 deterministic fallback。

mode 由现有 runtime config typed loader读取，非法值 fail closed 回到 `static_fusion` 并产生
error/status 诊断；不得用环境变量隐式覆盖 plan。`remem status --json` 显示 configured/effective
mode、policy version、gate report fingerprint/staleness 与 fallback reason。rollback 改回
`static_fusion`，不迁移/删除 DB 或 artifact。

default gate 输入必须绑定当前 commit、binary/version、dataset hash、fixture count、router policy、
plan schema 与 report schema。缺失、stale、样本不足、任一 threshold fail 或 verifier fail 都输出
`keep_static`。

### 8. Per-intent eval 与 ablation

新增 `src/eval/retrieval_router/`，使用 temp `REMEM_DATA_DIR`、seeded corpus 和 deterministic fake
channel failure/latency inputs。fixture 每个 intent 都包含：

- relevant canonical evidence；
- irrelevant semantic neighbor；
- stale/superseded evidence；
- project/branch mismatch；
- no-evidence abstention；
- channel timeout/error；
- generated enrichment 与 corrupted counterpart；
- unknown-intent control。

报告按 intent 与 overall 输出 Recall@k、nDCG、abstention accuracy、stale-followed rate、
irrelevant injection count/rate、token estimate、p50/p95 latency、memory-hurt。ablation 同一 corpus
分别执行 `static_fusion` 与 `intent_router`，禁止读取两个不同 baseline。

`eval/gates/baseline.json` 与 `thresholds.json` 增加所有声明维度；threshold 在报告生成前固定。
默认门禁至少要求：

- 所有目标 intent 改善达到各自 threshold；
- 任一非目标 slice 退化不超过 max drop；
- `memory_hurt` 不增加；
- unknown fallback 不低于 static；
- p95 latency 不超过预算；
- corrupted enrichment policy leak 为 0。

阈值的具体数值由 baseline 产出和 maintainer spec approval 固定；spec 不伪造尚未运行的数字。

### 9. Benchmark artifact

memory/coding run artifact 增加 versioned `retrieval_contract`：

- mode、resolved intent、policy version、plan hash；
- ContextAudit digest / schema version；
- degraded/abstention；
- dataset、implementation 与 policy fingerprint。

artifact verifier 对 `intent_router` condition 强制这些字段并复算/比对 hash/digest；static condition
可显式记录 `mode=static_fusion`，不能用空 router fields。coding benchmark 继续使用现有
`memory_hurt` 语义，不因 router 新增另一种互不兼容指标。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001` | router domain/intent/planner | schema JSON snapshots；六 intent mapping/fallback tests |
| `B-002` | planner/hash validation | repeated/parallel property tests；clock/env/DB-free test；invalid request tests |
| `B-003` | router executor/channel adapters | disabled-channel spy；plan mutation/hash mismatch；fail-closed partial execution tests |
| `B-004` | request filters + existing eligibility readers | project/branch/as-of/suppression/trust cross-scope fixtures |
| `B-005` | MCP `context_bundle`, REST `/api/v1/context` | explicit-wins、invalid enum/4xx/tool error、schema parity、legacy search compatibility tests |
| `B-006` | channels/fusion/audit | one-entry-per-channel、limit/cap/timeout/skip/fail-closed tests |
| `B-007` | high-risk execution/abstention | trusted-only、canonical Top 1、raw fallback exclusion、empty audited bundle tests |
| `B-008` | GH-933 binding + fusion dedupe | corrupted/duplicate projection matrix；zero generated-only high-risk Top 1 |
| `B-009` | GH-851 adapter | on/off/N/k、timeout/error baseline-order preservation、no candidate expansion tests |
| `B-010` | ContextBundle executor/audit | exact JSON snapshot；bundle/audit/executor hash equality；channel reason coverage |
| `B-011` | benchmark artifact types/verifier | missing/stale/mismatched hash/fingerprint negative fixtures |
| `B-012` | retrieval-router eval suite | six intent slices + unknown/empty/error fixtures；offline determinism test |
| `B-013` | ablation/default gate | same-head/dataset guard；each threshold fail fixture keeps static |
| `B-014` | runtime mode/status | static compatibility、explicit-only、default-on、invalid config、rollback tests |
| `B-015` | executor/fusion pagination | parallel repeat、stable tie-break、page snapshot、projection dedupe/idempotency tests |
| `B-016` | all adapters/error surfaces | no-LLM/network spy；timeout/cancel/DB/provider error visibility tests |
| `B-017` | docs/PR lifecycle/preflight | current contract/packet sync；PR body `Refs #934`；closure audit only after full acceptance |

## 数据流

```text
MCP context_bundle / POST /api/v1/context / offline eval
  -> parse versioned ContextRequest + optional explicit intent
  -> normalize project/cwd without widening caller scope
  -> retrieval_router::planner::plan
  -> validate schema + policy + plan_hash
  -> DB-backed ChannelExecutor (enabled channels only)
  -> canonical/projection identity validation
  -> limit + weight + contribution cap + fusion
  -> eligibility/source-anchor/trust/freshness
  -> optional GH-851 rerank
  -> canonical-top1 + abstention + token budget
  -> ContextBundle
  -> ContextAudit / safe adapter response
  -> eval/benchmark artifact records audit digest + fingerprints
```

生产路径不持久化 plan 或 bundle，不修改 canonical memory。benchmark/eval 只写调用方指定的 artifact
目录；默认 MCP/REST 只返回响应。日志不得包含 memory 正文。

## 备选方案

- 只扩展现有 GET/MCP search：无法自然表达多 section `ContextBundle` 与完整 typed request，且会把
  rollout 复杂度塞入兼容 endpoint，拒绝。
- 在每个 adapter 内复制 intent mapping：会产生 policy drift 与 hash 不一致，拒绝。
- 用 LLM 分类 intent：不可重复，可能扩大 scope/降低 trust，并新增 foreground 网络依赖，拒绝。
- 直接把 generated enrichment 当 FTS/vector 文本：会双重加权并失去 attribution，拒绝。
- 在没有 ablation 的情况下直接 default-on：不满足 issue 的 memory-hurt/latency/unknown gate，拒绝。

## 风险

- Security：intent、role、risk 或 scope adapter 错配可能越权检索；通过 typed DTO、显式优先、
  existing eligibility readers、high-risk canonical/trust gate 与 negative fixtures 控制。
- Compatibility：PR #940 叠在 #932 且版本 surfaces 有冲突；必须先刷新 base、在原 PR 处理，不以
  force push 或替代 PR 绕过。旧 search surfaces 保持 shape/default。
- Performance：15 channel 可能放大查询；disabled channel zero-call、candidate cap、timeout、
  contribution cap、p95 gate 与 no-required-network 约束控制。
- Data quality：projection corruption/duplication会污染排序；canonical binding、projection dedupe、
  contribution cap 与 high-risk disable 控制。
- Maintenance：ContextPlan、RetrievalPlan、Bundle audit 可能形成双 hash；字段明确区分，
  router bundle 只把 retrieval plan hash作为 top-level execution identity，schema snapshot 防漂移。
- Eval validity：小样本或 stale report 可误触 default；same-head/dataset fingerprints、最小样本、
  verifier 与 fail-closed gate 控制。

## 测试计划

- [ ] Unit：planner/schema/hash、channel mapping、executor zero-call、fusion/cap/dedupe、risk/trust、
      rerank fallback、audit/hash equality、runtime mode。
- [ ] Integration：temp DB 全 channel、GH-933 corrupted enrichment、MCP/REST explicit intent、
      ContextBundle JSON、status/capability、static compatibility。
- [ ] Eval：六 intent golden、unknown fallback、static-vs-router ablation、memory-hurt、latency、
      default-gate negative matrix、artifact verifier。
- [ ] Repository：`cargo fmt --check`、`cargo check`、focused tests、`cargo test`、
      `cargo clippy --all-targets -- -D warnings`、JS tests、version sync、full PR preflight。

## 回滚方案

1. default gate 未通过时不切换；继续 `static_fusion`，显式 Context Bundle surface 可保持
   `explicit_only`。
2. default-on 后出现回归时，把 typed rollout mode 改回 `static_fusion`，保留 plan/audit/benchmark
   artifact 供诊断；不删除或改写 memory DB。
3. 若新 endpoint/tool 本身不安全，先从 capability map 标记 unavailable 并返回明确错误，再用
   forward-fix；不得静默回退到跨 scope/static 查询。
4. schema/policy 回滚必须拒绝新版本 artifact，而不是把未知字段当旧成功；若 version surfaces 已
   发布，使用新 patch version 修复。

## Planned Changes Manifest

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 934,
  "complete": true,
  "paths": [
    "specs/GH934/product.md",
    "specs/GH934/tech.md",
    "specs/GH934/tasks.md",
    "docs/specs/GH934/PRODUCT.md",
    "docs/specs/GH934/TECH.md",
    "docs/specs/README.md",
    "README.md",
    "docs/ARCHITECTURE.md",
    "CHANGELOG.md",
    "src/retrieval_router.rs",
    "src/retrieval_router/domain.rs",
    "src/retrieval_router/intent.rs",
    "src/retrieval_router/planner.rs",
    "src/retrieval_router/executor.rs",
    "src/retrieval_router/executor/channels.rs",
    "src/retrieval_router/executor/fusion.rs",
    "src/retrieval_router/executor/audit.rs",
    "src/retrieval_router/executor/tests.rs",
    "src/retrieval_router/tests.rs",
    "src/context_bundle/domain.rs",
    "src/context_bundle/audit.rs",
    "src/context_bundle/executor.rs",
    "src/context_bundle/tests/executor.rs",
    "src/context_bundle/tests/schema.rs",
    "src/context/query.rs",
    "src/context/implicit_query.rs",
    "src/context/hybrid_context.rs",
    "src/context/types.rs",
    "src/memory/retrieval_enrichment.rs",
    "src/memory/retrieval_enrichment/tests.rs",
    "src/memory/service/types.rs",
    "src/memory/service/search.rs",
    "src/retrieval/search/memory/runner.rs",
    "src/retrieval/search/memory/text.rs",
    "src/retrieval/search/memory/weights.rs",
    "src/retrieval/search/memory/explain.rs",
    "src/retrieval/search/memory/tests.rs",
    "src/retrieval/rerank/stage.rs",
    "src/retrieval/rerank/types.rs",
    "src/retrieval/rerank/tests.rs",
    "src/git_trace.rs",
    "src/workstream.rs",
    "src/runtime_config.rs",
    "src/mcp/types.rs",
    "src/mcp/server/context_tools.rs",
    "src/mcp/server/tests.rs",
    "src/api/server.rs",
    "src/api/types.rs",
    "src/api/handlers.rs",
    "src/api/handlers/context.rs",
    "src/api/handlers/capabilities.rs",
    "src/api/tests.rs",
    "src/cli/context_types.rs",
    "src/cli/actions/context_plan.rs",
    "src/cli/actions/query/status.rs",
    "src/cli/actions/query/status/types.rs",
    "src/cli/actions/query/status/tests.rs",
    "src/eval.rs",
    "src/eval/retrieval_router.rs",
    "src/eval/retrieval_router/types.rs",
    "src/eval/retrieval_router/fixture.rs",
    "src/eval/retrieval_router/runner.rs",
    "src/eval/retrieval_router/tests.rs",
    "src/eval/gates.rs",
    "src/eval/gates/tests.rs",
    "src/eval/bench_artifact/types.rs",
    "src/eval/bench_artifact/verify.rs",
    "src/eval/bench_artifact/tests.rs",
    "src/eval/coding_bench/types.rs",
    "src/eval/coding_bench/artifact.rs",
    "src/eval/coding_bench/condition.rs",
    "src/eval/coding_bench/run_plan.rs",
    "src/eval/coding_bench/runner.rs",
    "src/eval/coding_bench/tests.rs",
    "eval/retrieval-router/fixtures/v1.json",
    "eval/retrieval-router/baseline.json",
    "eval/retrieval-router/thresholds.json",
    "eval/retrieval-router/report.json",
    "eval/gates/baseline.json",
    "eval/gates/thresholds.json",
    "eval/public/schemas/memory-run.schema.json",
    "eval/public/schemas/coding-run.schema.json",
    "eval/public/memory/suites/retrieval-router/suite.json",
    "eval/public/memory/manifests/retrieval-router-v1.json",
    "eval/public/memory/reports/retrieval-router-v1.json",
    "eval/public/memory/artifacts/retrieval-router-v1/",
    "eval/public/reports/baseline.json",
    "eval/public/reports/baseline.md",
    "Cargo.toml",
    "Cargo.lock",
    "plugins/remem/.codex-plugin/plugin.json",
    "plugins/remem/runtimes/remem-releases.json",
    "npm/remem/package.json",
    "server.json"
  ],
  "spec_refs": [
    "specs/GH934/product.md",
    "specs/GH934/tech.md",
    "docs/specs/GH934/PRODUCT.md",
    "docs/specs/GH934/TECH.md"
  ]
}
-->

本文件与 `tasks.md` 不构成 `spec_approval` 或 implementation 授权。原 PR #940、duplicate-work、
trusted base/path evidence、sensitive enforcement 与 human readiness/spec approval 全部通过前，不得
开始任何生产修改。
