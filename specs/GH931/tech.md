# Tech Spec

## Linked Issue

GH-931

## Product Spec

`specs/GH931/product.md`

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 931,
  "complete": true,
  "paths": [
    "specs/GH931/product.md",
    "specs/GH931/tech.md",
    "specs/GH931/tasks.md",
    "workflow.yaml",
    ".gitignore",
    "README.md",
    "README.zh-CN.md",
    "CHANGELOG.md",
    "docs/ARCHITECTURE.md",
    "docs/specs/README.md",
    "docs/specs/GH931/PRODUCT.md",
    "docs/specs/GH931/TECH.md",
    "docs/specs/issue385-coding-agent-ab/PRODUCT.md",
    "docs/specs/issue385-coding-agent-ab/TECH.md",
    "docs/specs/public-memory-benchmark/PRODUCT.md",
    "docs/specs/public-memory-benchmark/TECH.md",
    "eval/coding-bench/README.md",
    "eval/coding-bench/benchmark-charter.json",
    "eval/coding-bench/conditions.json",
    "eval/coding-bench/curated-file-budgeted-protocol.md",
    "eval/coding-bench/fixtures/tasks.json",
    "eval/coding-bench/schemas/conditions.schema.json",
    "eval/coding-bench/schemas/curator-log.schema.json",
    "eval/coding-bench/schemas/flagship-run.schema.json",
    "eval/coding-bench/schemas/flagship-report.schema.json",
    "eval/coding-bench/schemas/evidence-source-manifest.schema.json",
    "eval/coding-bench/schemas/live-run-approval.schema.json",
    "eval/coding-bench/examples/curator-log.example.json",
    "eval/coding-bench/validate_schemas.py",
    "eval/coding-bench/live-run-approvals.json",
    "eval/coding-bench/evidence/flagship-e2e-v1/run-records.jsonl",
    "eval/coding-bench/evidence/flagship-e2e-v1/source-manifest.json",
    "eval/coding-bench/reports/flagship-e2e-v1.json",
    "eval/coding-bench/reports/flagship-e2e-v1.md",
    "eval/claims/claims-registry.schema.json",
    "eval/claims/registry.json",
    "eval/claims/claim_gate.py",
    "src/eval/coding_bench/artifact.rs",
    "src/eval/coding_bench/approval.rs",
    "src/eval/coding_bench/condition.rs",
    "src/eval/coding_bench/failure.rs",
    "src/eval/coding_bench/fixture.rs",
    "src/eval/coding_bench/isolation.rs",
    "src/eval/coding_bench/run_plan.rs",
    "src/eval/coding_bench/runner.rs",
    "src/eval/coding_bench/score.rs",
    "src/eval/coding_bench/tests.rs",
    "src/eval/coding_bench/types.rs",
    "src/cli/eval_types.rs",
    "src/cli/actions/eval.rs",
    "src/cli/tests_eval.rs",
    "scripts/ci/check_public_claims.py",
    "Cargo.toml",
    "Cargo.lock",
    "npm/remem/package.json",
    "plugins/remem/.codex-plugin/plugin.json",
    "plugins/remem/runtimes/remem-releases.json",
    "server.json"
  ],
  "spec_refs": [
    "specs/GH931/product.md",
    "specs/GH931/tech.md",
    "docs/specs/GH931/PRODUCT.md",
    "docs/specs/GH931/TECH.md",
    "docs/specs/issue385-coding-agent-ab/PRODUCT.md",
    "docs/specs/issue385-coding-agent-ab/TECH.md",
    "docs/specs/public-memory-benchmark/PRODUCT.md",
    "docs/specs/public-memory-benchmark/TECH.md"
  ]
}
-->

该 manifest 是 GH-931 从 spec、runner implementation 到 official
report/wording 的累计路径边界，不要求单个 PR 触及全集。每个 linked PR 的
路径必须是其子集，最终 closure audit 对固定 baseline 后的 linked PR union
做精确核对。若实现证明需要新路径，先更新本 manifest 并重新取得
spec/security approval；spec lane 不预创建或伪造 report。

`workflow.yaml` 是必须先落地、单独复审的 sensitive-registry prerequisite，
不是允许普通实现 lane 顺手改治理配置。该 prerequisite 必须把
`specs/GH931/*`、live approval schema/registry、`approval.rs` 和 public-claim
authority 纳入 `enforcement.sensitive_registry`；在它合入 default branch 前，
不得实现 live authorization、运行 official matrix 或发布 claim。因为
`workflow.yaml` 本身已是 trusted sensitive path，本 complete manifest 的
approved-tech classification 必须为 `enforcement_sensitive=true`；当前只改
`specs/GH931/*` 的 spec PR 则仍应声明 `enforcement_sensitive=false`。

## Codebase Context

以下事实已在
`origin/main@5627a74942a41f51bdc03518fce726dbf1b46098` 核对：

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
| Public claim CI | `scripts/ci/check_public_claims.py` | 只基于旧 public baseline gate 扫 public surfaces。 | 接入 GH-931 hash-bound verdict，属于 enforcement-sensitive 路径。 |
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
- `conditions.json` 的 `runner_status` 随实现从 pending/scaffold 更新为
  `implemented`；validator 对 Rust/registry ID drift fail closed。

### 2. Isolation、fixture 和 live boundary（B-005-B-006、B-014-B-020）

- `CodingBenchRunIdentity` 绑定 task、condition、run index、attempt、
  fixture/prompt/score/model/timeout/sandbox hash 和 randomization seed。
- 每次 run 创建新的 repo、HOME、CODEX_HOME、REMEM_DATA_DIR 和 artifact
  root；env 从 allowlist 构造。真实 HOME/session/config/memory path 不挂载。
- provider/host auth 只由 live coordinator 在显式授权后复制最小必要 material
  到 private root；报告只记录 bootstrap ref/digest，不记录 secret bytes。
- hidden score files 在 agent 进程退出、sandbox 卸载后才写入；score command
  继续使用 argument array，不经过 shell 拼接。
- dry-run 构造并验证完整 plan、paths 和 capability declaration，调用图在
  auth/provider/agent spawn 前结束。普通 CI 只跑 dry-run、schema 和 synthetic
  tests。
- artifact 采用 temp-write + fsync + atomic rename；`attempt_id` 唯一，完成
  文件不覆盖。resume 只接受 hash 验证后的完成 tuple。
- live `run` 的唯一授权源是 `origin` default branch 上的
  `eval/coding-bench/live-run-approvals.json`，调用方不得传任意 registry path
  或自己选择 approval 内容。每个 entry 通过
  `live-run-approval.schema.json`，绑定 merged approval PR、APPROVED maintainer
  review node/reviewer association、merge commit、approved/expires 时间、
  canonical entry digest、exact code/fixture/claim-registry/model/timeout/
  sandbox hashes、允许的 matrix/tuple selectors、credential bootstrap ref
  （不含 secret bytes）以及累计 agent/LLM/cost hard caps；`approval_id` 由
  canonical digest + review node + merge commit 派生。
- `approval.rs` 必须在任何 auth read、network/provider/agent spawn 前通过
  GitHub API fresh 验证 approval PR 已 merge 到 default branch、reviewer 具备
  maintainer association、remote registry blob/digest 与本地 entry 完全一致。
  离线、过期、review 被 dismiss、registry drift、hash/tuple/cap 不匹配都
  fail closed；CLI 传入的 cap 只能进一步收紧，不能扩大授权。
- 每条命令生成独立 `execution_id`，但 canonical owner/repo +
  derived `approval_id` 唯一定位 append-only usage ledger；CLI/env/output root
  不得覆盖 ledger identity/location。ledger 使用跨进程锁、hash chain、
  temp-write/fsync/atomic rename，并与同 approval 的 immutable attempt/run
  artifacts 取累计消耗的最大一致值。resume、新 `execution_id`、并发命令和
  拆单都继承已消费 calls/cost；ledger missing/rollback、artifact reconciliation
  不一致或累计上限耗尽均在外部调用前失败。只有新的 maintainer-approved、
  default-branch-merged approval entry 能增加预算。

### 3. `remem_e2e` production adapter（B-008-B-010、B-017、B-021-B-022）

- fixture history episode 转为 production `CaptureEventInput`，使用
  `record_captured_event*` 写 isolated project/session 的真实
  `captured_events`。gold fields只供 post-run verifier，不作为写入内容。
- 按 production Stop/rollup contract enqueue `extraction_tasks`，使用真实 worker
  processing path和配置的 LLM provider做 extraction、candidate 与
  review/promotion policy。bounded drain 超时或任务失败即记录对应 stage，不
  降级。
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

- curator session 只接收按 `reference_time_epoch` 排序的 history episodes 和
  budget，target prompt/hidden score/gold facts 不进入其输入。
- curator 结束后 validator 检查 elapsed minutes、字符/token 上限、操作计数
  与冲突记录，计算并冻结 `MEMORY.md` SHA-256。
- target run 只挂载 frozen `MEMORY.md`，再次计算 hash 与 curator log 比对。
  `CuratedFileExpert` 继续使用现有 target-aware `curated_context`，但只能作为
  diagnostic。
- human maintenance claim 只接受人工 curator log；若为自动 curator，artifact
  必须标记不同 actor 类型且不得用于 70% human-maintenance comparison。

### 5. Failure taxonomy、report 与 claims（B-017-B-029）

- `failure.rs` 将 execution outcome 与 memory-stage diagnosis分开：每个 memory
  failure 恰好一个 stage/code，使用 registry 的 6-stage / 12-enum。
  无足够 evidence 时 suite error 显式为 unclassified，不推测。
- flagship run schema 固定 run identity、attempt history、condition surface、
  runtime/score result、stage failure、tokens/wall time、curator cost、
  memory-harm 和 attribution DAG/hash。
- official evidence writer 将每个 scanner-passed sanitized attempt/run 追加到
  `eval/coding-bench/evidence/flagship-e2e-v1/run-records.jsonl`，并生成
  `source-manifest.json`：记录 144 个 matrix keys、全部 attempt hashes、schema/
  code/fixture/registry hashes、scanner verdict、denominator policy、bundle hash
  和 report input hash。manifest 通过独立 schema；raw stdout/stderr/auth/
  private roots 只留在 ignored local artifacts，不能进入 bundle。
- report builder 先验证 144 tuple completeness、attempt policy、同 pair hashes
  、source manifest 与 bundle hash，再汇总 task-level denominator。缺失
  metric 使用 `null` 与 `missing_count`。aggregate-only report 或只位于
  `/tmp` 的 run output 不允许成为 public claim supporting evidence。
- paired bootstrap 以 task 为 resampling cluster，固定 algorithm/version/seed，
  分别计算 E2E vs no-memory superiority 与 E2E vs budgeted non-inferiority；
  run repetition 只在 task cluster 内聚合。
- 首个 official run 前把 registry 的 dataset/model/timeout/runs/exclusions/
  thresholds/bootstrap 固定并设 `locked=true`。任何 official artifact 的
  pre-registration digest 不匹配都 invalid。
- claim gate 先检查 matrix/artifact/stop-loss，再检查 effect/CI/cost。report
  hash、registry supporting report 与 Markdown/JSON 必须一致。
- `check_public_claims.py` 在原 baseline policy基础上读取 GH-931 registry；
  非 hash-bound PASS 时拒绝 public surface 上的 flagship superiority wording。

### 6. CLI、文档与版本

稳定命令：

```text
remem bench coding --suite issue385-v1 --matrix primary --dry-run \
  --json-out <path>
remem bench coding --suite issue385-v1 --matrix primary \
  --approval-id <id> --confirm-live-run --max-agent-calls <n> \
  --max-llm-calls <n> --max-estimated-cost-usd <usd> --json-out <path>
remem bench coding --suite issue385-v1 --condition remem_preloaded ...
remem bench coding-report --input <artifact-root> \
  --json-out <path> --markdown-out <path>
```

- 实际 CLI 名称可在实现时遵守现有 `BenchAction::Coding/Report` 结构，但必须
  保留显式 matrix、output、live confirmation 和 hard caps 语义；若路径变更，
  先更新 planned-path manifest。
- README/docs 在 report PASS 前只描述方法与无公开 outcome；report 通过后也
  只能引用 gate 允许且带 report link 的 wording。
- 用户可见 ID/CLI 变更在一个 integration PR 中按 version-sync contract 同步
  Cargo/plugin/npm/server/CHANGELOG。official report 可在后续不改 runtime
  version 的 evidence PR 中提交。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| B-001-B-004 state/IDs/matrix | types、run plan、registry validator | 新 ID parse 正例；旧 ID 负例；primary dry-run `planned_runs == 144`；缺/重 tuple 被拒绝。 |
| B-005-B-006 pair/randomization | run identity、planner | same-pair hash equality；fixed seed 可复现；模型/prompt drift 失败。 |
| B-007 no-memory | condition/isolation | hooks/MCP/SessionStart/file/native surfaces 全无；额外 surface 负例失败。 |
| B-008-B-010 E2E path | capture/extraction adapter、worker drain、context render | isolated integration test 证明 capture→use DAG；seed/save/preload shortcut 测试失败；provider/drain failure 不降级。 |
| B-011-B-013 curator/diagnostics | curator adapter、condition registry | target-blind/freeze/hash/budget 正负例；diagnostic 不进入 primary。 |
| B-014-B-016 isolation/security | isolation、score | unique private roots；host path/secret/hidden leak scanner；hidden file agent phase 不存在。 |
| B-017-B-020 failure/retry/dry-run | artifact store、approval verifier/ledger、runner | fault injection、immutable attempt、resume、duplicate/partial/hash drift negatives；伪造/未 merge/未 APPROVED/过期/drift approval 与 ledger rollback/并发超额/新 execution ID negatives；dry-run mock 零 spawn/network/auth。 |
| B-021-B-022 attribution/taxonomy | artifact/ref resolver/failure | 每类 ref 缺失、跨 run/project、unknown stage/code 失败；每 failure 恰好一个 stage/code。 |
| B-023-B-024 report/bootstrap | sanitized evidence bundle/source manifest、report builder | 144 records/hash/referential completeness；failure 留分母、null missing、task-cluster golden CI、unpaired/hash drift insufficient；从 bundle 独立重算得到相同 report input hash。 |
| B-025-B-029 claim gates | claim registry/gate/public CI | effect/CI/non-inferiority/cost/stop-loss 边界、自报 PASS、hash drift、非 PASS 正向 wording 全部覆盖。 |
| B-030 human gates | CLI/live authorization/handoff | 缺 approval/confirm/hard caps 在 agent/provider 调用前失败；spec 不自批。 |

## 数据流

```text
16 versioned tasks + locked registry
  -> isolated randomized 144-tuple plan
  -> condition setup
       no_memory: no memory surface
       curated_file_budgeted: blind curator -> frozen MEMORY.md + cost log
       remem_e2e: captured_events -> extraction_tasks -> worker
                  -> candidate/review/promotion -> memory/projection
                  -> production SessionStart/MCP retrieval
  -> target coding agent
  -> hidden deterministic scoring
  -> failure-stage + attribution resolution
  -> immutable run artifact/hash
  -> scanner-passed sanitized run-record bundle + source manifest
  -> task-level paired aggregation/bootstrap
  -> stop-loss + claim gate
  -> hash-bound JSON/Markdown report
  -> public wording CI + human release decision
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
  cargo fmt --check
  cargo check
  cargo clippy --all-targets -- -D warnings
  cargo test
  python3 scripts/ci/check_plugin_version_sync.py
  ```

- [ ] Live manual verification: only after SP931-T9 receives separate
  auth/network/cost approval; first smoke one task per primary condition, then
  full 144. Smoke artifacts are excluded from official denominator.

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

GH-931 当前 `ready_to_spec` 已获 maintainer 授权；本文件不构成
`spec_approval`。实施还需要 exact product/tech diff approval、
`ready_to_implement`、enforcement-sensitive planned-path approval 和当时
exact head 的 implement route evidence。live auth/provider/cost、最终 PR
review、claim wording、merge、issue close 和 release 仍需分别批准。
