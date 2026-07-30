# Tech Spec

## Linked Issue

GH-931

## Product Spec

`specs/GH931/product.md`

## Implementation Scope Manifest

根据 current-main governance，root `specs/GH931/` packet 是历史规划证据，
current implementation contract 以 `docs/specs/` 下对应文件为准。下面是
GH-931 从 contract、runner 到 official report/wording 的累计 implementation
scope reference；它不要求单个 linked PR 触及全集，也不是自动 allowlist、
执行前置条件或授权机制。linked PR 应记录实际触及的清单子集；最终 closure
audit 对固定 baseline 后的 linked PR union 与任何已说明的 scope deviation
做精确核对。

Contract 与用户文档：

```text
specs/GH931/product.md
specs/GH931/tech.md
specs/GH931/tasks.md
README.md
README.zh-CN.md
CHANGELOG.md
docs/ARCHITECTURE.md
docs/specs/README.md
docs/specs/GH931/PRODUCT.md
docs/specs/GH931/TECH.md
docs/specs/issue385-coding-agent-ab/PRODUCT.md
docs/specs/issue385-coding-agent-ab/TECH.md
docs/specs/public-memory-benchmark/PRODUCT.md
docs/specs/public-memory-benchmark/TECH.md
```

Benchmark registry、fixture、schema 与 evidence：

```text
.gitignore
eval/coding-bench/README.md
eval/coding-bench/benchmark-charter.json
eval/coding-bench/checkpoint-protocol.md
eval/coding-bench/conditions.json
eval/coding-bench/curated-file-budgeted-protocol.md
eval/coding-bench/{evaluation-clock-scope.json,production-drain-scope.json}
eval/coding-bench/fixtures/rekor-v2/{trusted-root.json,signing-config.json,bundle.json}
eval/coding-bench/fixtures/tasks.json
eval/coding-bench/schemas/checkpoint-receipt.schema.json
eval/coding-bench/schemas/conditions.schema.json
eval/coding-bench/schemas/curator-log.schema.json
eval/coding-bench/schemas/flagship-run.schema.json
eval/coding-bench/schemas/flagship-report.schema.json
eval/coding-bench/schemas/evidence-source-manifest.schema.json
eval/coding-bench/schemas/live-run-approval.schema.json
eval/coding-bench/examples/checkpoint-receipt.example.json
eval/coding-bench/examples/curator-log.example.json
eval/coding-bench/validate_schemas.py
eval/coding-bench/live-run-approvals.json
eval/coding-bench/evidence/flagship-e2e-v1/frozen-control-content.jsonl
eval/coding-bench/evidence/flagship-e2e-v1/run-records.jsonl
eval/coding-bench/evidence/flagship-e2e-v1/source-manifest.json
eval/coding-bench/reports/flagship-e2e-v1.json
eval/coding-bench/reports/flagship-e2e-v1.md
eval/claims/claims-registry.schema.json
eval/claims/registry.json
eval/claims/claim_gate.py
```

Rust runner、production clock readers 与 CLI：

```text
src/eval/coding_bench.rs
src/eval/coding_bench/checkpoint.rs
src/eval/coding_bench/checkpoint/{client.rs,proof.rs,tests.rs,trust.rs}
src/eval/coding_bench/{artifact.rs,approval.rs,condition.rs,failure.rs,fixture.rs,isolation.rs}
src/eval/coding_bench/{provider_adapter.rs,run_plan.rs,runner.rs,score.rs,tests.rs,types.rs,verified_exec.rs}
src/bin/remem-bench-supervisor.rs
src/clock.rs
src/lib.rs
src/context.rs
src/context/{types.rs,render.rs,render_inputs.rs,query.rs,audit.rs,fact_labels.rs}
src/context/{hybrid_context.rs,prompt_submit.rs}
src/context/render/eval.rs
src/mcp/server.rs
src/mcp/{mod.rs,types.rs}
src/mcp/server/{benchmark.rs,errors.rs,runtime.rs,search_tools.rs,context_tools.rs}
src/memory/{types.rs,dedup.rs,graph_contract.rs,lifecycle.rs}
src/memory/dedup.rs
src/memory/dedup/{access.rs,funnel.rs,hash.rs}
src/memory/graph_contract.rs
src/memory/edge.rs
src/memory/lifecycle.rs
src/memory/procedure/{mod.rs,trace_store.rs}
src/memory/service/types.rs
src/memory/store/write.rs
src/memory_candidate.rs
src/memory_candidate/{apply.rs,review.rs,review/approval.rs}
src/graph_candidate/{mod.rs,review.rs,source.rs,conflict_bridge.rs,tests.rs}
src/graph_candidate/tests/review_regressions.rs
src/db/{capture.rs,capture/extraction_task.rs,extraction/enqueue.rs}
src/extraction_worker.rs
src/observation_extract.rs
src/memory/{lesson.rs,preference.rs}
src/memory/{preference/query.rs,preference/render.rs,service/search.rs,store/read.rs}
src/db/observation.rs
src/retrieval/search.rs
src/retrieval/search/memory.rs
src/retrieval/search/memory/runner.rs
src/retrieval/search/memory/listing.rs
src/retrieval/search/memory/text.rs
src/retrieval/search/memory/text/graph.rs
src/retrieval/search/memory/source_anchor.rs
src/retrieval/search/memory/usage_rank.rs
src/retrieval/search_multihop.rs
src/retrieval/search_multihop/search.rs
src/retrieval/search_multihop/expand.rs
src/retrieval/rerank.rs
src/retrieval/vector.rs
src/retrieval/vector_candidates.rs
src/retrieval/memory_search.rs
src/retrieval/memory_search/fts.rs
src/retrieval/memory_search/like.rs
src/retrieval/entity.rs
src/retrieval/entity/search.rs
src/retrieval/entity/search/runner.rs
src/retrieval/entity/search/lookup.rs
src/retrieval/entity/search/sql.rs
src/retrieval/graph/query.rs
src/retrieval/graph/traverse.rs
src/retrieval/temporal.rs
src/retrieval/temporal/parse.rs
src/retrieval/temporal/fact_keys.rs
src/retrieval/temporal/fact_labels.rs
src/retrieval/temporal/search.rs
src/cli/eval_types.rs
src/cli/actions/eval.rs
src/cli/tests_eval.rs
```

Public-claim CI 与 version surfaces：

```text
scripts/ci/check_public_claims.py
scripts/ci/check_evaluation_clock_scope.py
Cargo.toml
Cargo.lock
npm/remem/package.json
plugins/remem/.codex-plugin/plugin.json
plugins/remem/runtimes/remem-releases.json
server.json
```

若实现证明需要新路径，在 linked PR 中说明原因，并同步 current
`docs/specs/GH931/` contract；本 historical packet 可随之刷新以保留追踪性，
但不机械阻止实现。maintainer 仍需人工 review exact scope/diff；涉及 live
credential、protected ledger、network isolation、public claim authority 或
security boundary 时还必须取得独立 security review。spec lane 不预创建或
伪造 report，也不授权 live run、public wording 或 merge。

## Codebase Context

以下事实已在
`origin/main@441d7da1325ab3b9eea39c10753d37131c062dbc` 核对：

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Current contract | `docs/specs/GH931/{PRODUCT.md,TECH.md}` | PR #936 scaffold 已落，runner execution 和 official runs 明确 pending。 | 本 packet 完成 follow-up，不重做 scaffold。 |
| Condition registry | `eval/coding-bench/conditions.json` | 三个 primary 已声明；`remem_e2e=pending_src_support`、`curated_file_budgeted=artifact_schema_only`，旧 diagnostic 仍映射 runner ID。 | registry truth 与 Rust 现状必须收敛。 |
| Rust condition | `src/eval/coding_bench/types.rs`, `condition.rs` | 仅有 `NoMemory/Remem/CuratedFile`；`Remem` 直接 seed DB 并追加完整 details。 | rename old path 为 diagnostic，并另建真实 E2E。 |
| Runner/plan | `runner.rs`, `run_plan.rs` | dry-run 当前计划 16×3×3=144，但三条件是旧 IDs；仅 Codex runner。 | v1 数量不变，condition 意义必须变为 primary 闭集。 |
| Artifact/report | `artifact.rs`, `failure.rs` | 报告有 outcome/citation 字段和旧 failure reasons，尚无 official attempt/hash/curator/failure-stage contract。 | B-017-B-029 要求完整、不可覆盖、可配对证据。 |
| Fixture | `eval/coding-bench/fixtures/tasks.json` | 16 deterministic tasks 均有 history episodes 与 score commands。 | 复用 task/outcome，不把 episode 中 gold memories 直接 seed 到 E2E。 |
| Capture/extraction | `src/db/capture.rs`, `src/db/extraction.rs`, `src/worker.rs`, `src/observation_extract.rs`, `src/memory_candidate.rs` | production path 提供 captured event、extraction task、worker 与 candidate/promotion side effects。 | E2E adapter 必须调用生产边界并记录 refs，不能复制简化管线。 |
| Context output | `src/context/render/eval.rs`, `src/context.rs` | `session_start_eval_snapshot` 可在 isolated DB 上渲染生产 SessionStart 结果。 | E2E retrieval 出口；不得追加 benchmark gold details。 |
| Curator scaffold | `curated-file-budgeted-protocol.md`, curator schema/example | target-blind protocol 与 log shape 已定义，runner 未验证 freeze hash/预算。 | 接入真实 budgeted primary condition。 |
| Claim gate | `eval/claims/{registry.json,claim_gate.py}` | 三个 claim 均 `INSUFFICIENT`，registry `locked=false`，无 supporting report。 | 首个 official run 前锁定；无证据继续 fail closed。 |
| Public claim CI | `scripts/ci/check_public_claims.py` | 只基于旧 public baseline gate 扫 public surfaces。 | 接入 GH-931 hash-bound verdict，属于 security-sensitive manual review 路径。 |
| Live authorization | `eval/coding-bench/` | 当前没有 default-branch approval registry、approval schema 或跨命令 usage ledger。 | caller-provided ID/caps 不是授权；B-020/B-030 要求 trusted registry 与累计预算。 |

## 设计方案

### 1. Condition ID 与 artifact compatibility（B-001-B-006、B-013）

- `BenchCondition` 使用闭集：
  `NoMemory`、`CuratedFileBudgeted`、`RememE2e`、
  `RememPreloaded`、`CuratedFileExpert`，后两者仅 diagnostic。
- CLI/parser 删除裸 `remem` 与 `curated_file`；不加 alias。错误信息列出新 ID
  和 primary/diagnostic 分类。
- `BenchCondition::PRIMARY` 固定前三个，默认 primary run 只用它们；
  diagnostic 必须显式 `--condition` 或 `--matrix diagnostic`。
- 已提交的 `baseline.json` / `fix-remem-smoke.json` 保持 immutable historical
  artifact。loader 根据 schema/version 明确标记 legacy，禁止把旧 ID 翻译后
  混入 flagship report。
- T2 必须在一个始终可编译的 milestone 中同步迁移 `types` 与所有 exhaustive
  `artifact/condition/runner/tests` consumer，再按依赖顺序移交后续 owner；
  不允许留下“等下一 task 补 match arm”的红 build。
- `conditions.json` 的 `runner_status` 随实现从 pending/scaffold 更新为
  `implemented`；validator 对 Rust/registry ID drift fail closed。

### 2. Isolation、fixture 和 live boundary（B-005-B-006、B-014-B-020）

- `src/eval/coding_bench.rs` 是现有 child-module authority；T3 原子创建并声明
  `approval`、`provider_adapter`、`verified_exec`，T3A 顺序接收 parent 后再
  原子创建并声明 `checkpoint`。每个 handoff commit 都通过 `cargo check`；
  T6 才拥有 runner/integration-test wiring，禁止 duplicate module 绕过接线。
- `CodingBenchRunIdentity` 绑定 policy-derived `run_phase`、
  `matrix_namespace`、task、condition、run index、attempt、
  fixture/prompt/score/timeout/sandbox hash、registered condition-order position、
  seed/PRNG/version/permutation digest。official namespace 固定为
  `issue385-v1/official-v1`；smoke namespace 由 reviewed approval entry 分配，
  caller 不得传入或覆盖，且永远不能被 report builder 解释为 official。
  identity 还绑定 OS/security-owner-anchored host supervisor identity，以及从 approved commit
  reproducibly build 后放入 read-only content-addressed mount 的 remem digest、
  target agent executable/version/profile digest，以及分别 canonicalized 的
  extraction provider/model/prompt/reasoning、enrichment、review/promotion 和
  retrieval config/policy hashes；这些 pair fields 不得折叠为一个 `model`。
  identity 还绑定 registration 的 UTC `evaluation_as_of` 与 virtual-clock policy
  hash；harness 将同一时钟注入 production temporal parser、age/staleness、
  retrieval/rendering 与 target environment，禁止语义路径读取 wall-clock
  “now”。
- T4 新增 immutable `src/clock.rs::EvaluationClock`，并维护
  `eval/coding-bench/evaluation-clock-scope.json`。普通 SessionStart/
  PromptSubmit/MCP outer adapter 每次 request 只 snapshot 一次 system time；
  benchmark coordinator 只从 validated run identity 构造固定
  `evaluation_as_of`。clock 不进入 target 可控参数、env、process/thread global
  或 DB global。inner API 必须显式传 `&EvaluationClock`/bound epoch，不能提供
  隐式 system default。
- SessionStart 传播链固定为
  `condition -> context::session_start_eval_snapshot_at -> render/eval ->
  render -> render_inputs -> query -> hybrid/fact/lesson/preference -> audit`；
  `LoadedContext.render_reference_epoch` 是所有 render section 与 audit 的唯一
  reference time。MCP 链固定为
  `MemoryServer(clock) -> search/context tools -> memory service ->
  search/list/multihop -> temporal/fact/entity/vector/graph/usage/source-anchor/
  rerank/explain`。`get_observations` 的 fact validity 与 access feedback 也必须
  消费同一值，因为 `last_accessed_epoch` 会影响后续 usage ranking。
- 所有 benchmark-reachable memory expiry/filter SQL 使用 bound evaluation epoch，
  禁止 `strftime(..., 'now')`；relative date/current-year、current fact validity、
  lesson stale-after、summary age、preference expiry、graph validity、usage
  recency、source-anchor demotion、rerank 与 explain staleness 都有跨层固定时钟
  tests。`check_evaluation_clock_scope.py` 同时拒绝语义函数中的 `Utc::now()`、
  `Local::now()`、`SystemTime::now()` 与 SQLite `now`，并验证 machine-readable
  inventory 与 planned paths 一致。
- production capture 的 semantic event time、candidate/dedup、promotion、
  TTL/validity 与 graph writes 同样显式接收该 clock；queue lease/retry 和纯
  operation audit 保留 real clock，但必须证明不能改变最终 content/order。
- benchmark 使用独立 closed MCP router，`tools/list` 只有 `search` 与
  `get_observations`；search 禁止 raw fallback，detail 只接受本连接 search
  签发的 `source=memory` IDs。其他 tool/alias/field 在 DB access 前拒绝。
- 真实 approval expiry、ruleset/TUF freshness、budget、timeout、supervisor
  `CLOCK_MONOTONIC` duration、纯 operation-log/poisoning/embedding-write timestamp
  属于独立 security/operational clock，不能由 `evaluation_as_of` 控制，也不能
  影响 target-visible selection/content/order。当前 evaluation snapshot 以
  `invocation=None` 绕过 injection gate；scope inventory 必须锁定这个调用事实，
  后续若 benchmark 接入 gate，先将其 output-affecting clock 纳入 T4。
- 每次 run 创建新的 repo、HOME、CODEX_HOME、REMEM_DATA_DIR 和 artifact
  root；env 从 allowlist 构造。真实 HOME/session/config/memory path 不挂载。
- provider/host/GitHub authority auth 只由 coordinator/service 的独立 OS
  principal 持有。target agent 及其 tool subprocess 的 sandbox 只挂载 task
  repo，不继承 service env，也不能读 coordinator 的 auth、REMEM_DATA_DIR、
  ledger、artifact 或 private-root path；SessionStart/MCP 只通过最小权限 broker
  返回已审计 context，不暴露 DB/file socket。
- target namespace 拒绝 DNS、public/RFC1918/metadata 与 Unix sockets。
  当前 pinned Codex CLI 仍使用 HTTP `model_provider`，因此 T3 提供 namespace
  内 private-loopback adapter：仅 Codex 主进程可连接，adapter 自身无 network，
  只经 sandbox 前创建的 bounded inherited pipes 与 mount 外 provider broker
  交换 fixed-schema RPC。tool subprocess 的 loopback/public/Unix-socket 均由
  OS policy 拒绝；adapter 拒绝 URL/fetch/tool-tunneling。capability test 若不能
  同时证明 model call 可用和 child-tool 零网络，则该平台/agent version 不可
  live。task repo 是无 remote 的 detached fixture tree，且不含 public harness、
  prompt/gold/hidden scorer files；agent browser/public-fetch 与各类 tool canary
  必须失败。
- supervisor 从 client/agent/provider pipes 读取 bounded frames，先以 exact
  secret fingerprints + structured credential patterns 做 streaming
  detect/redact，确认 sanitized 后才允许写 artifact。命中 secret 时原 frame
  只留在内存并立即清零/丢弃，run fail closed；禁用 core dump，任何 raw/
  ignored/temp stdout/stderr 都不得落盘。
- agent 退出并卸载 sandbox 后，独立 scorer OS principal/process 建立 read-only
  hidden tree；controller 永不 import/exec patched code。另一个不可信 code-worker
  principal 只挂载 validated patch tree、无 hidden/oracle mount，并只经
  supervisor pipe 上 bounded、closed-schema RFC 8785 JCS JSON RPC 接收输入和
  返回结果。scorer 拒绝 symlink/hardlink/device/path collision，并只比较
  scorer-owned oracle 与绑定 exact patch/tree/hidden digests 的 RPC 结果。
  stdout、exit 0、visible tests 或 worker 自报不能定 PASS；monkeypatch/shared
  interpreter、extra/truncated RPC、异常、crash 或 timeout 全部 fail closed。
- dry-run 构造并验证完整 plan、paths 和 capability declaration，调用图在
  auth/provider/agent spawn 前结束。普通 CI 只跑 dry-run、schema 和 synthetic
  tests。
- artifact 采用 temp-write + fsync + atomic rename；`attempt_id` 唯一，完成
  文件不覆盖。任何 human curator/reviewer、billable preparation 或 target work
  前，supervisor 先将
  `(run_phase,matrix_namespace,matrix_key,attempt_id,projection_hash,
  budget_receipt,timing_policy)` 的 `pre_target_work_started` CAS append 到
  anchored protected remote ledger。human interaction 的 supervisor-owned
  `CLOCK_MONOTONIC`
  start/end、actor assignment、input/output digest 与 frozen surface digest
  继续 append；caller/curator 不能提交 elapsed。crash/abandon recovery 追加
  `abandoned_before_target`，按已观察 duration 或 reviewed interaction ceiling
  中较保守者计费并封闭 run index，禁止 fresh-clone 重做。target spawn 前再将
  reservation receipt 与 `target_started` durable append；process 启动失败也
  视为 started failure。recovery 对 remote started 且无 terminal record 的
  attempt 以 compare-and-swap 追加唯一
  `abandoned_after_target_start` terminal record（`resolved=0`）并封闭 matrix
  key；不得再 spawn。scanner 先冻结不含 terminal attestation/checkpoint、
  source-manifest/report hash 或任何自身 digest 派生字段的 receipt-free RFC 8785
  JCS payload。trusted supervisor 计算 `payload_sha256` 后，才将
  `(matrix_key,attempt_id,payload_sha256,outcome,cost/timing hashes,
  frozen_control/treatment hashes)` 作为 terminal attestation CAS append；
  receipt 产生后由 source manifest detached mapping 绑定 payload、attestation
  与 checkpoint。resume/report 按 payload→ledger signature/ancestry→checkpoint
  →mapping 顺序验证，任何缺失或自引用都失败。
- live `run` 的唯一 policy 源是 `origin` default branch 上的
  `eval/coding-bench/live-run-approvals.json`，调用方不得传任意 registry path。
  entry 的 canonical policy preimage 明确排除 `approval_key`、review node、
  merge commit、承载它的 blob/tree/commit OID 和 mutable usage；
  `approval_key = sha256(repo_id || approval_pr_number ||
  canonical_policy_digest)`，字段在 review 前可知且不自引用。policy 绑定
  approved/expires 时间、exact executable/profile/fixture/
  `registration_projection`/timeout/sandbox hashes、policy-derived
  `run_phase`/`matrix_namespace`、allowed tuples、credential-bootstrap ref
  （无 secret bytes）和累计 agent/LLM/cost hard caps。policy 还绑定 exact
  ledger ref/genesis、ledger-writer GitHub App installation/repository identity、
  signature algorithm/key ID/public-key digest、update-authority 与 no-bypass
  integrity 两个 ruleset 的 ID/hash，以及 approval-pinned Sigstore TUF
  `TrustedRoot`/`SigningConfig` digests、Rekor operator/log IDs、accepted API/
  DSSE entry versions 和 minimum reviewed Rekor bundle/checkpoint。
  policy 还内嵌 immutable canonical pricing snapshot：`currency=USD`、provider/
  model SKU、effective timestamp、input/output/cache/tool-token 单价、每个
  call-kind 的最大 input/output/cache/tool tokens 与 decimal 向上取整 scale。
  currency conversion 不允许；SKU/price 变化必须新建 reviewed policy。
- verifier 启动一个隔离的 authority broker：仅该 broker 可读取最小
  repo-scoped GitHub credential 并访问 approval/ruleset/ledger read APIs；它不得读取
  provider/host/ledger-writer credential、启动 agent 或执行 benchmark。它 fresh
  验证 approval PR 的
  approved head tree 中 registry blob 的 canonical policy digest 与 key 匹配、
  PR 已 merge 到 default branch、
  APPROVED review 未 dismissed 且 reviewer association 符合 maintainer。
  provider/host auth 由另一个 principal 独立 bootstrap；authority broker 的
  credential 不进入该 principal、agent 或 artifact，但 broker 保持隔离存活，
  并在每个 reservation/transition/terminal seal 前重新验证 approval expiry、
  merge/review、registry blob、两个 ruleset、TUF trust、Rekor bundle chain 与
  ledger ancestry。GitHub/Rekor unavailable、过期、drift、hash/tuple/cap mismatch
  均 fail closed。
- ledger append 由另一个隔离 OS principal 上的 dedicated ledger-writer broker
  独占。其 reviewed GitHub App installation/repository/ref identity、record
  signature algorithm 与 public key ID 固定在 approval policy；短期 credential
  不能被 authority broker、provider principal、runner、agent 或 tool 读取。
  exact `refs/heads/remem-live-ledger` 同时受两个 active ruleset 保护：
  update-authority ruleset 只启用 `Restrict updates`，并只把该 App 列为唯一
  bypass actor；用户、admins、Actions、其他 apps 和通用 automation 都不在
  bypass list。integrity ruleset 的 bypass list 为空，并对所有 actor restrict
  deletion、block force push、require signed commits。writer App 对前者的
  bypass 不能绕过后者，也不能作为 ledger correctness 证据；authority broker
  仍拒绝任何 non-fast-forward、Rekor chain 不连续或 signature 不符的 history。
  每个 append commit 包含 canonical signed
  record envelope（repo/ref、schema version、monotonic sequence、parent OID、
  previous record digest、approval/policy digest、event payload digest、
  writer key ID/signature）；authority broker 必须从 pinned public key 验证完整
  signature/digest chain，才可接受 reservation、transition、terminal seal 或
  report input。
- cumulative budget 的 trust root 是 repo-scoped protected
  `refs/heads/remem-live-ledger` 与 GitHub 外部的 Sigstore Rekor public-good
  log，不是 clone-local ref/reflog、GitHub organization audit log、git common
  dir 或自建但未 provision 的 service。T3A 的
  `checkpoint-protocol.md`/schema/example/`checkpoint.rs` 固定 digest-only
  signed DSSE payload、Sigstore bundle、inclusion/consistency proof、TUF key/
  shard rotation、retry/recovery 与 privacy contract；receipt 固定
  `view_assurance=operator_consistency_only`，且只提交
  `(repo,ref,sequence,tip,ledger_digest,previous_bundle_digest)`，不提交 artifact、
  prompt、credential 或 private evidence。
  active Rekor shard URL 与 operator/log public keys 必须从 approval-pinned TUF
  `TrustedRoot`/`SigningConfig` 验证后发现，禁止硬编码会轮换的 endpoint。每个
  Git CAS 后取得的 bundle 必须验证 entry payload/signature、inclusion proof、
  signed tree checkpoint、从 reviewed minimum checkpoint 的 consistency proof
  和相对 previous bundle 严格递增的 log index；其他 Rekor entries 可占用中间
  index，因此不要求全局 index 连续。CAS 成功而提交中断时，recovery 只能为该
  exact already-signed tip 补交/定位同 digest entry，未取得 verified durable
  bundle 前不得 dispatch；duplicate/forked sequence 或不同 payload 一律失败。
  authority broker 在 startup 及每个 reservation/attempt transition/terminal
  seal 前 fresh fetch remote ref、两个 rulesets、TUF metadata 与 latest Rekor
  bundle，验证 current tip 是 genesis、minimum reviewed tip 和 latest anchored
  tip 的完整后代，并验证从 anchored tip 到 current tip 的每个 record
  signature/sequence/parent/digest。Rekor/TUF unavailable、bundle/proof 缺失、
  checkpoint rollback/consistency-proof mismatch、tip 回退/分叉、ruleset/writer/signature/
  ancestry mismatch 均 fail closed；fresh clone 不能以 clone-local 或仅当前
  GitHub tip 重置历史。
  上述保证只覆盖相对 approval-pinned/previous-observed checkpoint 的 rollback、
  proof inconsistency 与 ledger fork；Rekor 是 external operator anchor，不是
  independent witness。未另行批准 witness/gossip quorum 时，不声称检测恶意
  log operator 向本客户端持续提供的自洽 split view。
- 每次 billable host/LLM call 前，service broker 不接受 caller cost，而以
  reviewed call-kind token ceilings × pinned rates 做 checked-decimal
  conservative calculation，并按 policy scale 向上取整。unknown SKU/rate、
  token ceiling 缺失、overflow 或 price snapshot drift 均 fail closed。broker
  在 dispatch 前用 canonical provider tokenizer 验证 serialized
  input/cache/tool payload，并把 output/reasoning/tool/cache ceiling 写入
  provider 的 hard-limit fields；API/SKU 不支持硬限制或无法在超限前中断时
  fail closed。runner 随后要求 authority broker 完成上述 fresh remote
  rulesets/TUF/Rekor/ancestry/signature validation，再请求 ledger-writer broker
  以当前
  remote ledger tip 为 parent 创建包含
  `(approval_key,reservation_id,call_kind,token_ceilings,pricing_snapshot_hash,
  computed_worst_case_cost,
  execution_id,tuple,attempt)` 的 append-only reservation commit，并以
  non-force fast-forward ref update 作为 compare-and-swap；并发 sibling update
  失败后必须 refetch/recompute。reservation durable 后才允许 call；完成后追加
  settlement commit。每次 remote CAS 成功后，writer 必须提交上述 Rekor DSSE
  checkpoint 并取得 verified durable Sigstore bundle；bundle durability 未确认
  时不得 dispatch 或接受 transition。
  settlement 记录 broker-metered actual
  tokens/cost 并验证每类 usage 不超过 ceiling；超限为 security/cost breach，
  停止后续 calls，但 crash/timeout/abandoned reservation 永久按 computed
  worst-case 计入累计值。
  ledger ref 缺失、non-FF/force history、reconciliation drift、
  reservation reuse 或预算耗尽均 fail closed。resume、新 clone/
  `execution_id`、并发和拆单共享同一 remote total；只有新的 independently
  reviewed policy 可增加预算。

### 3. `remem_e2e` production adapter（B-008-B-010、B-017、B-021-B-022）

- `fixtures/tasks.json` 的每个 required history episode 必须新增 bounded、
  answer-bearing `raw_events`：真实 role/content/tool name/tool input/tool output/
  timestamp/host boundary 的 sanitized payload，以及独立 gold refs。adapter 按
  registered `history_episodes`/`raw_events` 原始嵌套数组顺序 flatten，派生全
  projection 连续 `source_ordinal=0..N-1` 并写入 call content；timestamp 按
  ordinal 非递减且允许同秒，`event_id` 只作 identity、绝不排序。每个 ordinal
  恰好映射一个 `record_captured_event*` call，call index 与 inserted row ID
  严格递增；gap/duplicate/shuffle、timestamp 回退、event-ID sort、call/row
  inversion 都在 commit 前失败。summary/`memories[].text` 只供 post-run
  verifier；缺 raw answer evidence 的 task schema invalid。
- 每个 raw event 提供 stable event ID，并以 production
  `CaptureEventInput(task_kind=ObservationExtract)` enqueue；禁止
  `run_claimed_exact`（它会抑制 candidate follow-up）。isolated DB 上单个 normal
  `run_next` worker 按 registered
  ObservationExtract→MemoryCandidate→GraphCandidate closure drain 至
  quiescent；unexpected/failed/residual/capped task 直接失败，不启动 SessionRollup/
  UserContext/background job 或 native-memory side effect。
- E2E condition 编译期/测试期禁止调用
  `save_seed_memory`、`render_seeded_remem_context` 和手工 memory save surface。
  diagnostic `RememPreloaded` 保留现有函数但名字、artifact 与 prompt 都明确
  `preloaded`。
- drain 完成后从 DB 读取 captured event、extraction task、candidate/review、
  promoted memory/projection refs，随后使用 production
  `session_start_eval_snapshot`/retrieval 输出；不调用
  `append_benchmark_memory_details`。
- agent 的 cited/used refs 与 injection audit 关联，生成同 run/project 的
  capture→use DAG。断链、跨 project、未知 ref 或 selected evidence 缺失均使
  E2E contract failure。

### 4. `curated_file_budgeted` adapter（B-011-B-012）

- fixture schema 定义 closed `curator_input_projection` allowlist，只允许每个
  history `raw_event` 的 timestamp、role、sanitized content、tool name/input/
  output 与 host boundary；明确拒绝 `expected_memory_facts`、`memories`、
  gold refs、target prompt、hidden score/oracle 和 scorer metadata。runner 在
  curator 启动前 canonicalize/hash 并把 projection 作为唯一输入；artifact
  保存 projection/hash，verifier 从 fixture 独立重建 byte-identical projection。
- planner 以 task 为 blindness batch：三个 run index 的 control curator 与
  treatment reviewer assignment/projection 必须全部在该 task 首个 target reveal
  前完成并冻结。默认要求 batch freeze；若使用不同 actor，authority ledger
  必须证明 stable actor identity/assignment，且该 actor 从未接触该 task 的
  target/outcome。暴露 actor 不得参与后续 repetition preparation。
- curator/reviewer interaction 必须在 durable `pre_target_work_started` 后开始。
  trusted supervisor 用 `CLOCK_MONOTONIC` 记录 start/end 和 interaction ID，
  receipts 绑定 projection、actor assignment、operations 与 frozen output；
  schema 拒绝 actor-entered elapsed。crash/abandon 按 observed/approved ceiling
  保守计费并封闭 run index。
- curator 结束后 validator 检查 supervisor timing receipt、字符/token 上限、
  操作计数与冲突记录，计算并冻结 `MEMORY.md` SHA-256。sanitized exact UTF-8
  bytes 写入 append-only content-addressed
  `frozen-control-content.jsonl`（digest、length、canonical bytes）；run record
  只引用 digest，verifier 从 committed bytes 重算并检查 target 实际 mount。
- target run 只挂载 frozen `MEMORY.md`，再次计算 hash 与 curator log 比对。
  `CuratedFileExpert` 继续使用现有 target-aware `curated_context`，但只能作为
  diagnostic。
- human maintenance claim 只接受人工 curator log；若为自动 curator，artifact
  必须标记不同 actor 类型且不得用于 70% human-maintenance comparison。
- `remem_e2e` 也记录同一 task/session 时间轴上的人工 candidate review、
  rejection、editing 与 manual promotion minutes/actions。人工 reviewer 的唯一
  输入是 closed `treatment_review_input_projection`：只允许 pre-target
  candidate content、source provenance、conflict/quality signals 与 policy
  rubric；明确排除 target prompt、gold/expected refs、hidden/scorer data 和
  任何 target outcome。runner 在 target reveal 前 canonicalize/hash 该
  projection，review/promotion 全部完成后 freeze promoted projection 与 log；
  committed artifact 保存输入/输出 hashes、supervisor timing receipt 与 ledger
  interaction attestation，verifier 可重建。与 control 相同，同 task 三个
  repetition 必须 batch-freeze，或由 ledger 证明后续 actor 完全未暴露。target reveal 后
  的 review/edit/promotion 或 projection drift 使 run invalid。official run
  可以使用正常人工 review policy，但任何 intervention 都进入 treatment cost；
  缺 log 时 maintenance comparison `INSUFFICIENT`。若声明 zero-touch tranche，
  任一 manual intervention 直接使该 tranche invalid，不能记为 0 minutes。

### 5. Failure taxonomy、report 与 claims（B-017-B-029）

- `failure.rs` 将 execution outcome、root memory failure 与 downstream
  consequences 分开。先按
  Capture→Extraction→Consolidation→Retrieval→Context compilation→Reader/use
  搜索最早有充分 causal evidence 的 stage，恰好输出一个 root 6-stage/
  12-enum code；后续 missing/ignored signals 放入 `consequences`，不改变 root。
  无充分 root evidence 时 suite error 为 `unclassified`，不得按检查顺序猜测。
- flagship run schema 固定 `run_phase`/`matrix_namespace` canonical matrix key、
  run identity、registered clock/order fields、
  `pre_target_work_started`/human interaction/target transitions 与 receipt-free
  terminal payload；receipt-free payload 明确排除 terminal attestation/checkpoint receipt、
  source-manifest/report hash、payload 自身 digest 及任何由该 digest 派生字段；payload 必须包含
  condition surface、runtime/score result、stage failure、tokens/wall time、
  supervisor-timed curator/treatment cost、closed
  `treatment_review_input_projection` + input/output/freeze hashes、
  frozen-control content digest、`memory_harm_rules` digest、closed causal
  classifier result/reason 和 attribution DAG/hash；unknown fields fail closed。
  T2 独占 schema ownership，T4 只实现 condition producer，T6 验证/汇总，
  不得靠未授权 schema edit 接线。
- official evidence writer 将每个 scanner-passed sanitized attempt/run 追加到
  `eval/coding-bench/evidence/flagship-e2e-v1/run-records.jsonl`，并生成
  `source-manifest.json`：记录 144 个 matrix keys、全部 attempt hashes、schema/
  code/fixture/binary/profile/registration hashes、scanner verdict、denominator
  policy、bundle/report-input hashes，以及 detached
  `(matrix_key,attempt_id,payload_sha256) → (terminal attestation OID/digest,
  checkpoint receipt digest)` mapping。manifest 通过独立 schema；control refs
  必须解析到 committed `frozen-control-content.jsonl` exact sanitized bytes。
  默认 report verifier 完全离线，按序复验 JCS payload、execution-time ledger
  receipt/signature/ancestry、checkpoint proof、detached mapping 与 bundle/control
  digests，不访问 network，也不声称 current freshness。仅在 mutable evidence
  tree 内 self-hash 一致不构成可信。只有 streaming-redacted stdout/stderr 可进入
  artifacts；raw credential frames 落盘前丢弃，auth/private roots 永不进入。
- 显式 network-only freshness invocation 不改 report bytes，只输出
  authority-signed detached receipt，绑定 `report_sha256`、ledger tip、
  ruleset/TUF/Rekor digests、`observed_at` 与 `expires_at`。publication、closure、
  release gate 要求 exact-report receipt 未过期；network denial、stale/expired
  receipt、wrong-report binding 或 tip/ruleset/TUF/Rekor drift 全部 fail closed。
- report builder 先验证 144 tuple completeness、attempt policy、同 pair hashes
  、source manifest 与 bundle hash，再汇总 task-level denominator。缺失
  metric 使用 `null` 与 `missing_count`。aggregate-only report 或只位于
  `/tmp` 的 run output 不允许成为 public claim supporting evidence。
- 对每个 task/condition，先计算三个预注册 run 的 binary resolved arithmetic
  mean；target-started failure 记 0，pre-target missing/integrity invalid 使
  matrix insufficient。bootstrap 固定算法/version/seed，每个 replicate 从 16
  个 task IDs 有放回抽 16 个 cluster，并对每个抽中 task 重算 treatment-control
  三-run mean difference。superiority 要求 point estimate >=10pp 且 percentile
  95% lower bound >0；non-inferiority 要求 lower bound >=-3pp 且同 denominator
  的 treatment human-maintenance reduction >=70%。
- registry 分为 immutable `registration_projection` 与 mutable
  `result_bindings`。T6 只实现 schema/gate 并用 synthetic projection 测试；
  T7 完成 CLI、docs、version sync 且从 exact final implementation head
  reproducibly build/记录最终 remem 与 agent executable hashes 后，才独占
  `registry.json` 并在任何 official-fixture live smoke 前锁定
  dataset/version/hash、condition IDs、official
  `run_phase=official`/`matrix_namespace=issue385-v1/official-v1`、smoke
  namespace non-collision policy、pair fields、timeout/runs、estimand、
  failure/missing/stop-loss denominator、exclusions、UTC
  `evaluation_as_of`/virtual-clock policy、condition-order seed/PRNG algorithm+
  version/完整 canonical tuple permutation+digest、bootstrap algorithm/version/
  seed、threshold 和 wording templates。planner 必须从 registration 重算相同
  order；run 只绑定 projection digest。gate 后只更新
  result bindings 的 PASS/FAIL/INSUFFICIENT、report hash 和 exact
  maintainer-approved wording，不改变 projection digest。
- stop-loss verifier 固定扫描 48 个 `remem_e2e` tuples。registration 为每个
  task hash-bind scorer-only `memory_harm_rules` 闭集及 verifier
  algorithm/version；每条规则包含 source provenance fact/content hash、
  canonical cited/used event predicate、严格 happens-before、normalized tool
  action/patch/scorer-failure fingerprint、`evaluation_as_of` 下的
  stale/superseded bit 与 deterministic evaluation order，并分类为互斥的
  `memory_caused`、`independent_cause` 或 `no_wrong_action`。
  `no_wrong_action` 只能由完整 sealed trace 证明没有 wrong-action predicate
  命中。target repo/agent 不得读取规则。verifier 对每个 tuple 只消费
  ledger-sealed event、patch、scorer 与 attribution DAG evidence，并要求恰好
  一个 terminal classification：paired `no_memory resolved=1` /
  `remem_e2e resolved=0` 且 `memory_caused` 才计入 `memory_hurt`；任何
  `memory_caused` 的 stale/superseded rule 都计入
  `stale_memory_followed`，不依赖 no-memory outcome。零/多匹配、
  hash/evidence/trace 缺失或无法唯一分类输出 `ambiguous_causality` 并使 gate
  `INSUFFICIENT`。两个 denominator 恒为 48，不能删除 tuple 或填 0。
- claim gate 先检查 matrix/artifact/stop-loss，再检查 effect/CI/cost。report
  hash、registry supporting report 与 Markdown/JSON 必须一致。
- `check_public_claims.py` 在原 baseline policy基础上读取 GH-931 result
  bindings。非 hash-bound PASS 时拒绝正向 wording；PASS 时也必须逐条验证
  claim ID、report hash/link 与 maintainer-approved exact UTF-8 text 一致，
  任意未登记数字、范围扩大、条件省略或改写都失败。

### 6. CLI、文档与版本

稳定命令：

```text
remem bench coding --suite issue385-v1 --matrix primary --dry-run \
  --json-out <path>
remem bench coding --suite issue385-v1 --matrix primary \
  --approval-key <key> --confirm-live-run --max-agent-calls <n> \
  --max-llm-calls <n> --max-estimated-cost-usd <usd> --json-out <path>
remem bench coding --suite issue385-v1 --approval-key <key> \
  --verify-live-approval-only --json-out <path>
remem bench coding --suite issue385-v1 --condition <primary-id> \
  --task <task-id> --runs-per-condition 1 \
  --approval-key <key> --confirm-live-run --max-agent-calls <n> \
  --max-llm-calls <n> --max-estimated-cost-usd <usd> --json-out <path>
remem bench coding --suite issue385-v1 --condition remem_preloaded ...
remem bench report --root <artifact-root> \
  --json-out <path> --markdown-out <path>
remem bench report --root <artifact-root> --verify-current-freshness --freshness-receipt-out <path>
```

- `--verify-live-approval-only` 使用 authority credential 读取 default-branch
  registry、review/merge、two-ruleset/TUF/Rekor 与 exact binary/profile/tuple/cap
  bindings，但在 provider/host credential bootstrap、agent spawn 和 billable
  dispatch 前退出；输出 policy-derived `run_phase`、namespaces、exact tuples、
  caps、binary digests 与 `authorized` verdict。caller 字符串不能让它 PASS。
- verify/smoke/official/report/freshness 只由固定
  `/usr/local/libexec/remem-bench-supervisor` 执行。独立 security owner 将其
  provision 为 root-owned immutable service；approval 绑定 OS code-sign/
  fs-verity/unit measurement 与不可由 caller 使用的 attestation public key。
  supervisor 自 authority 取 expected digest（不接受 caller 值），再对 remem/
  agent `openat(O_NOFOLLOW)`、same-fd hash/fstat，并用 Linux
  `execveat(AT_EMPTY_PATH)` 或 reviewed equivalent 执行。artifact 验证其签名
  attestation；wrong supervisor/direct invocation 失败。平台不能保证时 fail
  closed，shell `shasum` 不是执行证明。
- 保留现有 `BenchAction::Coding/Report` 结构与 `bench report --root` command
  contract；新增参数必须保留显式 matrix、output、live confirmation、hard caps
  与 detached freshness receipt 语义。若路径偏离 implementation-scope reference，在 linked PR 说明并
  同步 current contract。
- README/docs 在 report PASS 前只描述方法与无公开 outcome；report 通过后也
  只能引用 gate 允许且带 report link 的 wording。
- 用户可见 ID/CLI 变更在一个 integration PR 中按 version-sync contract 同步
  Cargo/plugin/npm/server/CHANGELOG。official report 可在后续不改 runtime
  version 的 evidence PR 中提交。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| B-001-B-004 state/IDs/matrix | types、run plan、registry validator | 新 ID parse 正例；旧 ID 负例；primary dry-run `planned_runs == 144`；缺/重 tuple 被拒绝。 |
| B-005-B-006 pair/randomization | run identity、registered clock、planner | same-pair executable/profile + `evaluation_as_of` equality；registration 预先固定 order seed/PRNG version/完整 tuple permutation，planner 重算 byte-identical；真实 clock、order/seed/binary/profile drift 均失败。 |
| B-007 no-memory | condition/isolation | hooks/MCP/SessionStart/file/native surfaces 全无；额外 surface 负例失败。 |
| B-008-B-010 E2E path | source-ordinal raw fixture、capture/extraction adapter、run schema、worker drain、context render | nested-array ordinal 唯一排序且 call/row 严增；same-second 正例与 gap/shuffle/event-ID-sort/inversion 负例；answer-bearing capture→use DAG、blind batch、seed/save/preload/drain negatives。 |
| B-011-B-013 curator/diagnostics | curator projection/schema、supervisor timing、content-addressed control bundle | allowlisted projection 可重建；elapsed 只能来自 supervisor monotonic receipts；committed frozen bytes 可重算 mounted hash；跨-run target exposure、gold/target/hidden/scorer 注入、missing bytes/receipt、freeze/hash/budget 负例失败；diagnostic 不进入 primary。 |
| B-014-B-016 isolation/security | service/agent privilege split、deny-egress broker、streaming redactor、scorer/code-worker RPC | agent/tool 无 auth/DB/private-root/network/public-repo access；controller 不 import/exec patch，code worker 无 hidden mount；stdout/exit0/monkeypatch/shared-interpreter/RPC tamper 全失败。 |
| B-017-B-020 failure/retry/dry-run | parent-wired approval/checkpoint modules、pre-target/target/terminal transitions、pricing broker、per-call authority、anchored CAS ledger | human/prep 前 durable attempt；abandoned pre-target work 保留/保守计时且不可重做；dispatch 前验证 actual input 并硬设 output/reasoning/cache/tool caps；每次 fresh 重验 expiry/two-ruleset/TUF/Rekor/signature/ancestry；terminal digest seal；wrong writer、rules/log rollback/proof inconsistency、crash/race/超额 negatives 均失败，dry-run 零 external call。 |
| B-021-B-022 attribution/taxonomy | artifact/ref resolver/failure | 每类 ref 缺失、跨 run/project、unknown stage/code 失败；overlapping failures 按 earliest causal root 稳定，downstream consequences 单列。 |
| B-023-B-024 report/bootstrap | receipt-free JCS payloads、detached receipts/source manifest、report builder | 144 records；payload→ledger→checkpoint→mapping 可离线复验；freshness receipt 单独绑定 report；三-run mean、missing/failed rules 与 16-task bootstrap golden。 |
| B-025-B-029 claim gates | post-final-binary registration projection、closed causal-rule verifier、result bindings、gate/public CI | T6 synthetic registry 后 T7 才以 final binary/clock/order permutation 冻结 live registration；superiority/non-inferiority、48-run stop-loss、trusted treatment review cost、唯一 `memory_caused`/`independent_cause` 匹配正例，零/多匹配、sealed evidence/hash 缺失、post-smoke mutation 与 PASS wording/report-link negatives。 |
| B-030 human gates | CLI/live authorization/handoff | 缺 approval/confirm/hard caps 在 agent/provider 调用前失败；spec 不自批。 |

## 数据流

```text
16 versioned tasks + post-final-binary locked registry
  -> pinned evaluation clock + preregistered PRNG/permutation
  -> isolated 144-tuple plan
  -> durable pre-target attempt + task-wide blind preparation batch
  -> condition setup
       no_memory: no memory surface
       curated_file_budgeted: blind curator -> supervisor timing
                                -> content-addressed frozen MEMORY.md bytes
       remem_e2e: captured_events -> extraction_tasks -> worker
                  -> candidate/review/promotion -> memory/projection
                  -> production SessionStart/MCP retrieval
  -> per-call authority/two-ruleset/TUF/Rekor/ledger revalidation + token hard limits
  -> target coding agent
  -> hidden deterministic scoring
  -> failure-stage + attribution resolution
  -> registered closed memory-harm causal classifier
  -> immutable receipt-free JCS payload/hash -> authoritative terminal ledger seal
  -> scanner-passed bundles + detached source-manifest receipt mapping
  -> task-level paired aggregation/bootstrap
  -> stop-loss + claim gate
  -> hash-bound offline JSON/Markdown report -> detached network freshness receipt
  -> public wording CI + receipt-expiry gate + human release decision
```

只有 isolated temp roots、显式 artifact output、scanner-passed sanitized
run-record bundle/source manifest 和最终 sanitized report 可写。真实 HOME、
auth bytes、host session 与 hidden content 不进入可提交 artifact。

## 备选方案

- **把现有 `Remem` 直接改名为 `remem_e2e`**：它直接 seed gold memory 并追加
  full details，违反 B-008/B-009，拒绝。
- **保留旧 ID alias**：会让脚本和 report 无法判断 primary 与 diagnostic，
  违反 no-alias contract，拒绝。
- **用自动 curator 代替人工成本基线**：无法支持 human maintenance claim；
  可作为 diagnostic，但拒绝进入 primary cost comparison。
- **把 extraction 固定输出写入 fixture**：只验证管线外壳，不验证真实 LLM
  extraction，拒绝作为 official E2E；可用于单元测试 fault fixture。
- **CI 自动跑 144 个 agent/LLM runs**：隐式消耗 auth/成本，拒绝；CI 只验证
  offline artifacts。

## 风险

- **Security**：auth、真实 HOME/session 和 hidden tests 可能泄漏。使用 private
  roots、env allowlist、post-run scanner 和 secret-redacted artifacts，任一
  leak fail closed。
- **Logic**：shortcut、condition contamination、重试挑选和丢失分母会制造
  虚假增益。使用 condition闭集、pair hashes、immutable attempts 和 matrix
  completeness gate。
- **Statistical validity**：16 task clusters 样本较小。公开完整 CI；CI 含 0
  不作 superiority claim。
- **Compatibility**：旧 report 使用 `remem/curated_file`。保留为 legacy
  artifact，拒绝 silent alias translation。
- **Cost**：official primary 至少 144 agent runs，E2E 还含 extraction LLM。
  live authorization 必须绑定最大 calls/cost，resume 不能重置累计预算。
- **Maintenance**：production capture/extraction/context contract 可能变化。
  adapter 只调用 production boundaries，并用 exact code/fixture hash 绑定报告。

## 测试计划

- [ ] Scaffold/offline:

  ```bash
  python3 eval/coding-bench/validate_schemas.py
  python3 eval/claims/claim_gate.py check
  python3 eval/claims/claim_gate.py --self-test
  cargo run -- bench coding --suite issue385-v1 --matrix primary \
    --dry-run --json-out /tmp/gh931-plan.json
  jq -e '.planned_runs == 144' /tmp/gh931-plan.json
  ```

- [ ] Focused Rust:

  ```bash
  cargo test eval::coding_bench
  cargo test cli::tests_eval
  ```

- [ ] Public claims:

  ```bash
  python3 scripts/ci/check_public_claims.py --self-test
  python3 scripts/ci/check_public_claims.py
  ```

- [ ] Repository gates:

  ```bash
  GH931_EXACT_HEAD="$(git rev-parse HEAD)"
  test "$(gh pr view "$PR_NUMBER" --json headRefOid --jq .headRefOid)" = \
    "$GH931_EXACT_HEAD"
  python3 scripts/ci/check_pr_preflight.py \
    --base origin/main --pr-body-file /tmp/pr-body.md
  test "$(git rev-parse HEAD)" = "$GH931_EXACT_HEAD"
  cargo fmt --check
  cargo check
  cargo clippy --all-targets -- -D warnings
  cargo test
  python3 scripts/ci/check_plugin_version_sync.py
  ```

- [ ] Live manual verification: only after SP931-T9 receives separate
  auth/network/cost approval。首轮 smoke 固定为三个 disjoint canonical keys：
  `(smoke/<approval-pr>/no_memory, no_memory,
  ticket-key-memory-convention, 1)`、
  `(smoke/<approval-pr>/curated_file_budgeted, curated_file_budgeted,
  slug-normalizer-contract, 1)`、
  `(smoke/<approval-pr>/remem_e2e, remem_e2e,
  workstream-title-continuity, 1)`，tuple 依次表示
  `(matrix_namespace, condition, task, run_index)`。namespace 只能由 approval
  policy 派生；每个 key 用独立 `--condition` + `--task` +
  `--runs-per-condition 1` invocation。三者与
  `issue385-v1/official-v1` 不相交，但所有 reservation/terminal records 仍写入
  同一个 `refs/heads/remem-live-ledger` 并共享累计预算。每次 dispatch 前必须
  先完成 `--verify-live-approval-only`，验证 exact namespace/tuple 且
  `planned_runs == 1`。不得用 `--task-set smoke --matrix primary` 代替，因为其
  Cartesian product 是 9 tuples。三个 smoke 全部终态通过并获人工 review 后才
  可创建独立 full approval 并运行 144；smoke artifacts 排除在 official
  denominator 外。

## 回滚方案

1. 在 official run 前可撤回新 primary runner，保留 scaffold 与本 spec，并让
   registry 保持 `INSUFFICIENT`。
2. 若 E2E adapter 发现 shortcut/泄漏，停止后续 runs，保留 invalid artifacts
   审计；修复后提升 artifact/policy version，不覆盖旧 attempts。
3. 若 report/bootstrap/claim gate 有缺陷，撤回 public wording，保留原始
   artifacts 和旧 report hash，生成新版本报告。
4. runtime/CLI 回滚同步所有 version surfaces；旧 condition alias 不因回滚而
   被静默重新引入。

## Human Gates

本 historical packet 只提供 planning/reference evidence，不自动批准或阻止
implementation。每个 implementation lane 开始前由 maintainer 人工 review
current product/tech contract、actual scope 与 exact diff；live credential、
authority/ledger writer、network isolation、hidden scorer 或 public-claim
boundary 还需要独立 security review。任何 live smoke/official run 都必须在
exact executable/policy/caps 上取得单独的 auth/network/cost approval；public
wording 需要 maintainer 对 report-bound exact UTF-8 text 明示批准。最终 PR
review、merge、issue close 和 release 各自独立，测试或 preflight 通过不能替代
这些人工决定。
