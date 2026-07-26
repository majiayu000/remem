# Tech Spec

## Linked Issue

GH-935

## Product Spec

`specs/GH935/product.md`

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 935,
  "complete": true,
  "paths": [
    "specs/GH935/product.md",
    "specs/GH935/tech.md",
    "specs/GH935/tasks.md",
    "workflow.yaml",
    ".gitignore",
    "README.md",
    "README.zh-CN.md",
    "CHANGELOG.md",
    "docs/ARCHITECTURE.md",
    "docs/specs/README.md",
    "docs/specs/GH935/PRODUCT.md",
    "docs/specs/GH935/TECH.md",
    "docs/specs/public-memory-benchmark/PRODUCT.md",
    "docs/specs/public-memory-benchmark/TECH.md",
    "eval/cross-host/README.md",
    "eval/cross-host/benchmark-charter.json",
    "eval/cross-host/claims-registry.json",
    "eval/cross-host/schemas/cross-host-task.schema.json",
    "eval/cross-host/schemas/cross-host-run.schema.json",
    "eval/cross-host/schemas/cross-host-source-seal.schema.json",
    "eval/cross-host/schemas/cross-host-report.schema.json",
    "eval/cross-host/schemas/cross-host-claim-verdict.schema.json",
    "eval/cross-host/schemas/evidence-source-manifest.schema.json",
    "eval/cross-host/schemas/live-run-approval.schema.json",
    "eval/cross-host/live-run-approvals.json",
    "eval/cross-host/examples/run-artifact-valid.json",
    "eval/cross-host/examples/run-artifact-invalid.json",
    "eval/cross-host/scripts/schema_validate.py",
    "eval/cross-host/scripts/scan_artifacts.py",
    "eval/cross-host/scripts/run_dry.py",
    "eval/cross-host/evidence/cross-host-v1/primary-run-records.jsonl",
    "eval/cross-host/evidence/cross-host-v1/native-ablation-run-records.jsonl",
    "eval/cross-host/evidence/cross-host-v1/source-manifest.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-architecture-decision.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-branch-specific-truth.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-failed-attempt-lesson.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-git-evidence.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-multi-hop-relation.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-negative-constraint.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-prior-bug-root-cause.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-same-name-repo-isolation.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-stale-superseded-decision.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-unresolved-conflict-abstention.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-user-project-preference.json",
    "eval/cross-host/tasks/claude-to-codex/cc2cx-workstream-next-action.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-architecture-decision.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-branch-specific-truth.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-failed-attempt-lesson.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-git-evidence.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-multi-hop-relation.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-negative-constraint.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-prior-bug-root-cause.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-same-name-repo-isolation.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-stale-superseded-decision.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-unresolved-conflict-abstention.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-user-project-preference.json",
    "eval/cross-host/tasks/codex-to-claude/cx2cc-workstream-next-action.json",
    "eval/cross-host/reports/cross-host-v1.json",
    "eval/cross-host/reports/cross-host-v1.md",
    "eval/cross-host/reports/cross-host-v1-gate.json",
    "src/eval.rs",
    "src/eval/host_isolation.rs",
    "src/eval/coding_bench/isolation.rs",
    "src/eval/coding_bench/runner.rs",
    "src/eval/coding_bench/tests.rs",
    "src/eval/cross_host.rs",
    "src/eval/cross_host/types.rs",
    "src/eval/cross_host/fixture.rs",
    "src/eval/cross_host/isolation.rs",
    "src/eval/cross_host/condition.rs",
    "src/eval/cross_host/approval.rs",
    "src/eval/cross_host/runner.rs",
    "src/eval/cross_host/score.rs",
    "src/eval/cross_host/report.rs",
    "src/eval/cross_host/bootstrap.rs",
    "src/eval/cross_host/claim_gate.rs",
    "src/eval/cross_host/tests.rs",
    "src/cli/eval_types.rs",
    "src/cli/actions/eval.rs",
    "scripts/ci/check_public_claims.py",
    "Cargo.toml",
    "Cargo.lock",
    "npm/remem/package.json",
    "plugins/remem/.codex-plugin/plugin.json",
    "plugins/remem/runtimes/remem-releases.json",
    "server.json"
  ],
  "spec_refs": [
    "specs/GH935/product.md",
    "specs/GH935/tech.md",
    "docs/specs/GH935/PRODUCT.md",
    "docs/specs/GH935/TECH.md",
    "docs/specs/public-memory-benchmark/PRODUCT.md",
    "docs/specs/public-memory-benchmark/TECH.md"
  ]
}
-->

该 manifest 是 GH-935 所有 linked PR 的累计预期文件边界，不要求任一单独
spec/implementation/report PR 的 diff 等于全集。每个 PR 的 touched-path subset
必须属于该 manifest；T12 closure audit 从固定 baseline 起收集全部 linked PR 的
file lists，并对 union 做最终精确相等检查。若真实宿主 CLI 探测、fixture 设计或
实现证明需要其他路径，必须先用准确路径更新本 manifest、重新取得 human
spec/security review，再修改新增路径。candidate report 只有在真实运行
evidence 经 scanner/verifier 通过后才生成，但不得等待 claim gate PASS：
`PASS`、`FAIL`、`INSUFFICIENT` 的 candidate report/evidence 都必须保留。
claim gate 消费 immutable candidate report hash 并另写 gate result；本 spec
lane 不创建或伪造任何运行、报告或 verdict。

`workflow.yaml` 是必须先落地、单独复审的 sensitive-registry prerequisite，
不是普通实现 lane 可顺手修改的治理文件。它必须在 live authority/claim
implementation 前把 `specs/GH935/*`、approval schema/registry、
`src/eval/cross_host/{approval.rs,claim_gate.rs}`、claim verdict schema/result
与 public-claim authority 纳入 `enforcement.sensitive_registry`。因为
`workflow.yaml` 本身是 trusted sensitive path，本 complete manifest 的
approved-tech classification 必须为 `enforcement_sensitive=true`；当前只改
`specs/GH935/*` 的 spec PR 仍应声明 `enforcement_sensitive=false`。

## Codebase Context

以下 anchors 已在 `origin/main`
`5627a74942a41f51bdc03518fce726dbf1b46098` 核对：

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| 当前 suite 状态 | `eval/cross-host/README.md:7-9` | 明确为 `infrastructure_only_no_runs`，禁止引用结果。 | B-001 的当前 truth；实现不能把 dry-run 当 outcome。 |
| Charter/matrix | `eval/cross-host/benchmark-charter.json:20-37`, `91-104` | 固定四个 primary conditions、每 tuple 3 runs、stop-loss 和 paired report 要求。 | 保持 `cross-host-v1` 产品边界，并升级 executable/report contract。 |
| Task lifecycle | `eval/cross-host/schemas/cross-host-task.schema.json:43-46`, `99-153` | task 只有 `skeleton_todo/ready`，已有 score/gold/todo 结构；24 个文件当前均为 skeleton。 | 增强 fixture/source episode 合同并把 24 个 task 逐个变为真实可执行。 |
| Run artifact | `eval/cross-host/schemas/cross-host-run.schema.json:68-97`, `99-165` | 已有基础 metrics、isolation 和 attribution 字段，但 environment/artifacts 仍是松散 object，缺 matrix completeness、attempt、hash 和 report linkage。 | B-006、B-017-B-021 需要严格 schema 与 referential checks。 |
| 离线验证 | `eval/cross-host/scripts/run_dry.py:42`, `84`, `107`, `125` | 校验 task/artifact 并打印计划矩阵，从不启动宿主。 | 保持 CI/dry-run 无网络，并扩充 v2 lifecycle/matrix negatives。 |
| Leak scanner | `eval/cross-host/scripts/scan_artifacts.py:57`, `75`, `102`, `168` | 扫描 HOME/session/auth/private-root markers，并有 self-test。 | 扩充 hidden fixture、cross-run、source/target store 泄漏负例。 |
| 可复用隔离 | `src/eval/coding_bench/isolation.rs:8-14`, `33-79`, `81-100` | `CodexIsolation` 在 macOS 创建临时 HOME/CODEX_HOME、host-read sandbox 并检查 private markers；非 macOS fail closed。 | 抽取共享隔离 primitive，再增加 Claude Code adapter；不得复制一套更弱的隔离。 |
| Coding runner | `src/eval/coding_bench/runner.rs:142-187`, `363-397` | 每 run 创建临时 repo/data，调用 Codex，保存 stdout/stderr/diff，timeout 后评分。 | 复用 process-group timeout、artifact、hidden-test-after-agent 模式。 |
| 禁止的 primary shortcut | `src/eval/coding_bench/condition.rs:48-66`, `71-105` | 当前 diagnostic `Remem` 直接 seed memory 并追加完整 benchmark details。 | GH-935 `remem_shared` 必须另建真实 capture/extraction path，不能复用该 shortcut。 |
| 现有 CLI | `src/cli/eval_types.rs:3-13`, `45-91`; `src/cli/actions/eval.rs:12-34` | `bench` 只有 verify/memory/coding/report，无 cross-host 子命令。 | 增加明确 `bench cross-host` run/verify/report surface，不另造隐藏脚本入口。 |
| Eval module | `src/eval.rs:1-18` | 公开现有 eval modules，无 cross_host 或共享 host isolation module。 | 新模块从这里接线。 |
| 通用 claim registry | `eval/claims/claim_gate.py:101-149` | 校验 wording、status、supporting report existence/hash，但不计算 paired bootstrap/stop-loss。 | 可复用 registry/hash/wording contract；GH-935 仍需专用 deterministic result gate。 |
| Public claim surface | `scripts/ci/check_public_claims.py:16-21`, `66-87`, `98-135` | CI 扫 README/CHANGELOG，但只读取现有 public baseline gate。 | 接入 hash-bound cross-host gate，避免未经批准的跨宿主结论。 |
| Public benchmark contract | `docs/specs/public-memory-benchmark/TECH.md:96-120` | 允许 Rust eval 实现；要求显式 report path，dry-run 不调用 agent。 | GH-935 作为第三类 cross-host outcome evidence 扩展该 current contract。 |
| GH935 current contract | `docs/specs/GH935/PRODUCT.md:36-47`; `TECH.md:36-42` | 明确基础设施-only，列出 executable fixtures、host harness、bootstrap/claim 和 export cost follow-ups。 | canonical packet 把这些 follow-ups 变为 gated tasks，不改写已完成历史。 |
| Live evidence/authorization | `eval/cross-host/` | 没有 report 目录、live-run approval schema/registry 或已执行 run；本次仅批准 spec 流程。 | live smoke/full matrix 必须在实现、独立复审和新的限额授权之后执行，不能继承本次授权。 |

## 设计方案

### 1. Contract 与 task v2（B-001-B-005、B-030）

- `benchmark-charter.json` 保持 suite id `cross-host-v1`，提升结构
  `schema_version`，状态按闭集
  `infrastructure_only_no_runs → executable_no_runs → partial_evidence →
  complete_evidence` 单向推进。状态由 verifier 从 artifact 计算，不能由
  report 自报。
- task v2 在每个现有 task 文件内增加确定性 `fixture_repo`（初始 branch、
  repo files、必要的同名 decoy repos）、可执行 source episode prompt 与
  source assertions。`history_episodes.memories` 只作为 expected/gold
  evidence，不作为任何 condition 的 DB seed 输入。
- 24 个 task 逐个人工填充确定性代码 fixture、source episode、target prompt、
  hidden files、score commands、required/forbidden patch patterns 和 gold
  facts；每 task 至少两个 chronological source episodes，以支持 exporter
  generation + update maintenance cycle。完成后才清空 `todo` 并设为 `ready`。
- source-seal schema 固定 source host transcript、tool events、git patch、
  episode boundaries、binary/model/profile hashes 和 content hash；同一
  `(direction,task,run_index)` 只生成一次 seal，conditions 只能消费 seal，
  不得各自重新运行 nondeterministic source episode。
- v1 skeleton 和旧 run schema 不做 silent migration。validator 可给出明确
  `schema_version unsupported` 或通过一次性、可审阅的 converter 产生新文件；
  原文件和 hash 保留。

### 2. 共享宿主隔离与 adapter（B-009-B-011、B-015-B-017、B-032）

- 从 `coding_bench/isolation.rs` 抽取 `src/eval/host_isolation.rs`：
  `PreparedHostIsolation`、env allowlist、host-read sandbox、private-root
  lifecycle、process-group cleanup 和 marker scan。现有 coding bench 迁移到
  同一 primitive，确保没有隔离回归。
- `cross_host/isolation.rs` 定义闭集 `HostKind::{ClaudeCode,Codex}` 和各自
  adapter。每个 phase 建立独立 HOME、host config/session root 和
  phase-private condition data root；source 与 target 严格串行复用同一个
  run-scoped canonical absolute workspace path。source seal/cleanup 后，该 path
  必须从 approved fixture 重建再启动 target，保持
  `project_id.rs` 的 canonical Git-root identity；same-name decoy 使用不同 path/
  project ID。只有 condition 明确允许的 sanitized
  credential bootstrap 可以复制到 private root，credential bytes 永不进入
  artifact。`remem_shared` 另建一个位于两侧 private roots 之外、按 run 唯一命名
  的 transfer store；source/target 只能串行把它挂载为 `REMEM_DATA_DIR`，其他
  condition、其他 run、session/config roots 都不能访问它。
- macOS 使用 host-read sandbox；其他平台在获得等价 deny-host-read 证据前
  fail closed。adapter 启动前记录宿主 binary/version/model/reasoning 和
  executable hash；未知 alias 或版本探测失败即停止。
- live `run` 必须同时带显式确认参数与 `--approval-key`。唯一 policy 源是
  `origin` default branch 的 `eval/cross-host/live-run-approvals.json`；调用方
  不能传任意 approval 文件/root。entry canonical preimage 明确排除
  `approval_key`、review node、merge commit 与 usage；
  `approval_key = sha256(repo_id || approval_pr_number ||
  approved_head_tree_oid || canonical_policy_digest)`，字段在 review 前可知且
  不自引用。policy 绑定 approved/expires timestamps、exact code/fixture/
  source-seal/config/model/host executable/profile hashes、allowed matrix/tuples、
  credential-bootstrap ref、hard caps 与 `decision=allow`。
- runner 先执行隔离 authority-only phase：仅该 phase 可读取 repo-scoped
  GitHub credential，fresh 验证 registry approved tree/blob、approval PR 已
  merge、APPROVED review 未 dismissed 且 reviewer association 符合 maintainer。
  它不能读取 host/provider auth 或启动 benchmark。清除 authority credential
  后才允许 host bootstrap；offline、remote drift、过期、hash/tuple mismatch
  均 fail closed。
- CLI 还要求调用方显式传入不高于 registry entry 的三项 hard caps。
  authoritative usage ledger 是 repo-scoped protected
  `refs/heads/remem-live-ledger`，不是 clone-local git common dir。每个 billable
  host/LLM call 前，以当前 remote tip 为 parent 写入包含 approval/reservation/
  tuple/call-kind/worst-case units/cost 的 commit，并用 non-force fast-forward
  ref update 做 compare-and-swap；concurrent sibling update 必须 refetch/
  recompute。reservation durable 后才允许 call，完成后追加 settlement；
  crash/timeout/abandoned reservation 永久按 worst-case 计费。force/non-FF
  history、reservation replay、cross-clone/execution ID split、reconciliation
  drift 或预算耗尽均 fail closed；只有新的独立 reviewed policy 可增加预算。
  `--dry-run`、verify、schema self-test 和普通 CI 的 call graph 不得读取 auth
  或进入 adapter spawn。
- source phase 完成后先终止进程、flush hook/capture、等待 bounded extraction
  drain、记录 evidence refs，再销毁 source session runtime；target phase
  只能随后启动。cleanup 失败产生 artifact 并阻止该 tuple 继续。

### 3. Condition engine（B-008、B-012-B-016、B-029）

`cross_host/condition.rs` 以闭集实现 memory surface，所有 condition 共享
fixture/prompt/scorer/source-seal/executable/model/profile hash：

- `no_memory`：hooks/MCP/remem/native/export 全部关闭。
- `target_host_native`：在 target prompt 揭示前，将同一 source seal 经
  target host 的真实 native-memory preparation/import surface ingest 到全新
  target HOME，再重新启动 target session；只允许目标宿主自己的 native store，
  记录 preparation tokens/time/turns。ingest 后 native store 为空、不可读取或
  target prompt 已揭示均 invalid，不能计作 native control。
- `exported_file`：第一 source episode 后 target-blind 生成 handoff，后续每个
  episode 后 update，记录 generation/maintenance tokens/time/turns/bytes/diff，
  最终 freeze。所有 conditions/hosts 使用同一 versioned
  `condition_context_envelope` broker/config；prompt 中仅有相同的 envelope
  protocol，不加 condition-only note。exported condition 将 frozen content
  放入 envelope，其他 condition 使用各自 surface/empty envelope。
- `remem_shared`：source adapter 使用生产 hooks/automatic capture 写入临时
  run-scoped transfer store，bounded worker drain 后先销毁 source 的
  HOME/config/session/condition roots，再由 target adapter 将同一 transfer store
  挂载为 `REMEM_DATA_DIR`，通过正常 SessionStart/MCP/Context Bundle 读取。
  surface manifest/hash 必须证明唯一共同路径就是该 transfer store，并拒绝任何
  source session root、phase-private condition root 或 cross-run path。禁止调用
  `coding_bench::condition::render_seeded_remem_context` 或
  `save_memory` shortcut。
- `remem_without_host_native_import` /
  `remem_with_host_native_import` 使用完整 144-tuple diagnostic plan。with arm
  通过 #852 importer 产生 pending/quarantined candidates 后，执行预注册、
  target-blind、独立 reviewer protocol；只把明确 approved candidates 提升到
  可检索 projection，记录 reviewer cost/decision/evidence，且始终保留
  `host_native_import` origin 与 non-canonical trust。without arm 执行相同
  reviewer schedule 的 empty control；除 import content/review outcomes 外
  config hash 必须一致。禁止把 quarantine 自动当作 active memory。
- oracle/full transcript/preloaded 等 diagnostic surfaces 有独立 condition id，
  report 层永不把它们加入 primary denominator 或 public comparison。

### 4. Runner、状态机与 artifact（B-006-B-008、B-017-B-021）

- `cross_host/runner.rs` 先构造 72 个
  `(direction,task,run_index)` source-stage keys，每 key 只运行一次 source host，
  原子封存 source seal 后才 fan out。随后构造随机化完整 tuple plan：
  primary 固定 `24×4×3=288`；native ablation 固定 `24×2×3=144`。missing、
  duplicate、seal mismatch 或 source re-execution 都 fail closed。
- 每个 tuple 具有稳定 `matrix_key` 和不可复用 `attempt_id`。artifact 先写临时
  文件，flush/fsync 后 atomic rename；完成 artifact 不可覆盖。
- pre-target infrastructure failure 可使用新 attempt 重试同一 tuple，历史
  artifact 保留；target 已启动后的 outcome failure 是该 run index 的最终
  outcome，不允许用“再跑一次成功”替换。report 采用预注册的 first-started-
  target attempt policy。
- resume 读取已验证 artifact hash，只补缺失 tuple。重复 matrix key、完成
  artifact hash 变化、半写文件或 run/config hash 不一致均 fail closed。
- `cross_host/score.rs` 在 target agent 退出后才 materialize hidden files，
  运行 array-argument score commands，验证 changed paths/patch patterns，
  并生成 failure taxonomy、metrics 与 attribution linkage。
- run schema 将 environment/artifacts 改为 closed object，并要求 suite/
  code/fixture/source-seal/config hash、phase status、attempt history、scanner
  result、scoring provenance、target-native/export/import-review cost 和
  attribution stage refs。每个 stage 必须是 `present(ref)` 或 typed
  `absent_due_to(failure)`；pipeline failure 的下游 absence 不排除 run，target
  started outcome 仍以 failure 留在 primary denominator。
- raw stdout/stderr/auth/private roots 留在 `.gitignore` 的本地 artifact
  目录。official evidence writer 分别将 288 个 primary attempts/runs 与完整
  native-import ablation 写入 scanner-passed sanitized JSONL bundles，并生成
  `source-manifest.json`，绑定全部 matrix/attempt hashes、schema/code/fixture/
  config/approval hashes、scanner verdict、denominator policy、bundle hashes
  与 candidate-report input hash。manifest 通过独立 schema；committed
  candidate report 只引用该 evidence/source-manifest，不引用 raw local paths。

### 5. Report、paired bootstrap 与 claim gate（B-022-B-031）

- `cross_host/report.rs` 先验证 primary/native-ablation source manifest、
  bundle hashes、matrix completeness 和每个 run schema/hash，再按
  方向/condition/task 汇总。所有适用失败进入分母；缺失 metric 输出 `null`
  加 `missing_count`，不写 0。它先生成 immutable candidate JSON/Markdown
  report 与 content hash，candidate 不含自引用 verdict，且无论后续
  `PASS`、`FAIL` 或 `INSUFFICIENT` 都保留。
- `bootstrap.rs` 用固定算法版本、显式 seed、95% CI 和 task-cluster
  resampling，对 `remem_shared` 分别配对 `target_host_native` 和
  `exported_file`。配对单位是同方向、同 task、相同 run-index config 的
  outcome cluster；方向分别计算，aggregate 只作为补充。
- `claim_gate.rs` 消费 candidate report hash 与 locked registry，依次检查：
  1. 288 primary completeness；
  2. 144 native import ablation completeness；
  3. artifact/scanner/attribution integrity；
  4. direction-specific paired CI；
  5. exported-file cost presence；
  6. 五项 stop-loss。
  leak predicates 扫描全部 288 primary tuples；`memory_hurt` 与
  `stale_memory_followed` 按 product B-027 在每方向固定 36、aggregate 72 个
  remem tuples 上计算。required attribution missing 使 verdict insufficient，
  不得缩 denominator。
  任一安全 leak/stop-loss 失败优先于 effect；CI 含 0 只生成预注册
  directional/insufficient wording。gate 把 `PASS`/`FAIL`/`INSUFFICIENT`、
  candidate report hash、registry hash、允许/禁止 wording 与 reason codes
  写入独立 immutable `cross-host-v1-gate.json`；不得改写或删除 candidate
  report 来隐藏失败。
- `claims-registry.json` 预注册两个方向的 remem-vs-native、
  remem-vs-exported 和 stop-loss claim，初始均为 `INSUFFICIENT`。gate 复用
  `eval/claims/claim_gate.py` 的 report-hash/wording contract，同时要求专用
  cross-host verdict 与 report hash 一致。
- `cross-host-report.schema.json` 固定 matrix counts、direction results、
  denominator、bootstrap config/CI、cost、ablation、stop-loss、code/fixture
  hashes 和 source artifact manifest；claim verdict 使用独立
  `cross-host-claim-verdict.schema.json`，避免 report/hash/verdict 自引用。
- `scripts/ci/check_public_claims.py` 读取 committed cross-host report/registry；
  没有 hash-bound PASS 时，README/README.zh-CN/CHANGELOG 中的正向跨宿主
  superiority wording 失败。普通 CI 只验证已提交 evidence，绝不执行 live
  benchmark。

### 6. CLI 与文档

新增稳定命令：

```text
remem bench cross-host verify --root eval/cross-host --json-out <path>
remem bench cross-host run --root eval/cross-host --runs-per-condition 3 \
  --matrix primary --json-out <path> [--dry-run]
remem bench cross-host run --root eval/cross-host --runs-per-condition 3 \
  --matrix native-import-ablation --json-out <path> [--dry-run]
remem bench cross-host run --root eval/cross-host --matrix smoke \
  --direction <direction> --task-id <task-id> --condition <condition> \
  --run-index <index> --approval-key <key> --confirm-live-run \
  --max-host-calls <n> --max-llm-calls <n> \
  --max-estimated-cost-usd <usd> --json-out <path>
remem bench cross-host report --root eval/cross-host \
  --json-out <path> --markdown-out <path>
remem bench cross-host gate --root eval/cross-host \
  --registry eval/cross-host/claims-registry.json \
  --report eval/cross-host/reports/cross-host-v1.json \
  --json-out eval/cross-host/reports/cross-host-v1-gate.json
```

- 所有写 report 的命令要求显式 output path。
- 任意非 dry-run `primary`/`native-import-ablation` 也必须提供
  `--approval-key`、`--confirm-live-run` 和三项 hard caps；approval registry
  entry 的 allowed tuple set 必须覆盖实际 plan，且 runtime counter 不能超过
  任何 cap。
- `--matrix smoke` 强制 direction/task/condition/run-index 各恰好一个，只产生
  一个 tuple，并在 schema/report 中永久标记 `excluded_from_public_denominator`。
  `verify --input <run-artifact> --approval-key <key> --expected-matrix smoke`
  重新校验 exact head/fixture/model、授权期限、调用/成本计数、sandbox、cleanup
  与 exclusion marker。
- `run --dry-run` 只验证 tasks、matrix、adapter availability declaration 和
  paths，不读取 auth、不启动宿主。
- `verify` 对 committed report、sanitized artifacts、hash、claim registry
  做离线验证；`gate` 是 `cross_host/claim_gate.rs` 的唯一 result-writer，
  原子写入 schema-valid gate JSON。通用 Python `eval/claims/claim_gate.py`
  只验证 registry/hash/wording，不接收未实现的 `--report/--json-out`。
- report/evidence task 对 PASS/FAIL/INSUFFICIENT 都必须更新
  `eval/cross-host/README.md`、`docs/specs/README.md` 和 canonical GH935/public
  benchmark contracts 的实际运行状态与 report/gate links。root README 只有
  report + gate PASS + exact wording approval 后才加入数字；否则保留无正向
  结论但 current contracts 不能继续声称 `executable_no_runs`。
- 代码引入用户可见 CLI，按 repo version-sync contract 在一个实现 PR 中同步
  Cargo/plugin/npm/server 版本和 CHANGELOG；真实 run/report evidence 可在后续
  非版本 PR 中提交。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| B-001 infrastructure/insufficient truth | charter state derivation、report gate | synthetic empty suite：`bench cross-host verify` 输出 `insufficient` 且非 PASS；README 无结果。 |
| B-002, B-005 task directions/categories | task schema、`fixture.rs` | `python3 eval/cross-host/scripts/run_dry.py` 证明两个方向各 12 类；wrong-host fixture 被拒绝。 |
| B-003, B-004 ready lifecycle/empty fields | task schema、schema self-tests | 24 files 为 `ready` 且 todo/score/fixture 完整；ready+empty-score 与 missing-key negative fixtures 失败。 |
| B-006 primary 288 | run plan、evidence source manifest、report completeness | dry-run 精确打印 288；删一个、复制一个、unverified 一个或 bundle hash drift 的 report tests 均 insufficient。 |
| B-007 native paired ablation | 144-tuple diagnostic plan、report | dry-run 精确输出 144；with/without import 同 hash 配对通过；缺侧、重复或 config drift negative tests 失败。 |
| B-008 comparability | one-source seal、matrix/config hashing | 每个 direction/task/run 只执行一次 source 并 fan out 同一 seal；source 重跑或 prompt/fixture/executable/model/profile 任一 drift 被 verifier 拒绝。 |
| B-009, B-032 explicit/human authorization | CLI、default-branch approval registry/schema、remote usage ledger、route/handoff | `--dry-run` mock 证明零 spawn/network；自引用 key、伪造/未 merge/未 APPROVED/过期/mismatched approval、authority credential 泄漏到 provider phase、换 clone/execution ID/root、reservation crash、CAS race、缺 confirm 或超全局 hard cap 均在 billable call 前失败；human gate tasks 未勾选。 |
| B-010, B-015 phase isolation/order | shared isolation、runner state machine | temp HOME/config/session/phase roots 全异；source/target 串行复用同一 canonical workspace 并保持 project ID；decoy 为不同 ID；`remem_shared` 仅允许当前 run transfer store；target 先启动、其他 shared path 和 source cleanup failure tests 均失败。 |
| B-011, B-016 leakage/hidden tests | sandbox、scanner、score timing | scanner self-test 覆盖真实 HOME/session/auth/private/hidden paths；agent 读取 hidden file 的 fixture 判 breach。 |
| B-012 condition surfaces | condition engine | target-native preparation 产生可读 native state、exported handoff 经共同 host-neutral envelope 被消费；任一条件退化成 no-memory 或出现额外 surface 的负例失败。 |
| B-013 real remem pipeline | remem_shared condition、capture attribution | integration test 从 hook event 到 selected context refs；production code test 断言未调用 seed/save/preload shortcut。 |
| B-014, B-024 export freeze/cost | exported-file generation/update protocol、report | 至少两 episode、首次 generation、逐 episode update、target-prompt-before-freeze 与缺 cost artifact 被拒绝；report 列出 per-task/aggregate 四类成本。 |
| B-017 failure completeness | runner/artifact schema | auth/crash/timeout/extraction/score/scanner/cleanup fault injection 均产生 typed failure 或 suite error。 |
| B-018, B-019 retry/resume | artifact store、attempt policy | retry 保留旧 artifact；overwrite、duplicate matrix key、partial file、changed hash tests 全部失败。 |
| B-020, B-021 attribution integrity | run schema、score/ref resolver | present ref 可解析；合法 typed downstream absence 保留 pipeline failure 于分母；无 failure 的缺 ref、跨 run ref、unknown/conflicting origin、native-as-canonical negative fixtures 均被拒绝。 |
| B-022, B-023 denominators/directions | sanitized primary/ablation bundles、source manifest、report builder | failure run 在分母；缺值为 null；单方向缺失时 aggregate 不得 PASS；从 committed bundles 独立重算得到相同 candidate input hash。 |
| B-025, B-026 paired bootstrap/CI wording | bootstrap、claim gate | fixed seed golden CI 可重现；unpaired/config drift/CI includes 0 只能得 directional/insufficient。 |
| B-027, B-028 stop-loss precedence | report/claim gate | leak 分母固定 288，hurt/stale 每方向 36/aggregate 72；精确 causal predicate 与阈值边界正负例；缺 attribution 为 INSUFFICIENT，resolved gain + 任一 leak 仍为 FAIL。 |
| B-029 native import trust | condition/attribution/report | 完整 144 with/without report 分开；import candidate 未经独立 review/promotion、origin/trust 被篡改或混入 primary 时失败。 |
| B-030 compatibility | versioned loaders | v1 skeleton/old artifact 被明确拒绝或转换后重新验证，不能直接 complete。 |
| B-031 public claim surface | immutable candidate report、Rust `bench cross-host gate`、CI check、post-live docs | candidate 先生成且 PASS/FAIL/INSUFFICIENT 均保留；真实 gate CLI 写 schema-valid result；report/gate hash mismatch、非 PASS + positive README wording 均使 `check_public_claims.py` 失败；任一 verdict 后 current contract/status/link 更新，PASS+approved wording/link 正例通过。 |

## 数据流

```text
versioned charter + 24 ready tasks
  -> dry-run / complete randomized matrix plan
  -> one source host isolated phase per direction/task/run
  -> source termination + bounded pipeline drain + immutable source seal
  -> fan out the same seal to target-native preparation / exported generation+updates /
     remem transfer / no-memory surface
  -> target host isolated phase at the same canonical workspace identity
  -> target-blind native-import review/promotion diagnostic
  -> hidden-test scoring
  -> leak scan + attribution resolution
  -> immutable sanitized primary/ablation bundles + source manifest
  -> direction-specific aggregation
  -> paired task-cluster bootstrap
  -> native-import ablation + exported-file cost + stop-loss
  -> immutable candidate JSON/Markdown report
  -> claim gate -> separate hash-bound PASS/FAIL/INSUFFICIENT result
  -> offline public-claim CI
  -> optional human-approved README wording
```

持久化仅发生在 benchmark 指定的 temp/private roots、显式 output path 和最终
sanitized report paths。真实 user HOME、默认 remem DB、来源宿主 session store
与 hidden fixtures 永不进入可提交 artifact。

## 备选方案

- **只扩展 Python dry-run**：无法安全复用现有 Rust process-group timeout、
  host-read sandbox、hidden-test scoring 和 CLI/report contracts，拒绝。
- **复用 coding bench 的 `Remem` condition**：该路径直接 seed memory 并追加
  gold details，只能作为 diagnostic upper bound，违反 B-013，拒绝。
- **只执行一个方向**：无法证明跨宿主对称性，也会掩盖单向失败，拒绝。
- **把 native import 混入 `remem_shared`**：无法归因其增益/伤害，拒绝；必须
  单独 paired ablation。
- **在 CI 自动执行 288+ live runs**：会隐式使用 auth/network/LLM，违反人工
  授权和 workflow maturity，拒绝。

## 风险

- **Security**：宿主 auth、真实 HOME/session、hidden tests 和 private fixtures
  是最高风险边界。使用 allowlist env、host-read sandbox、private root、
  post-run scanner、sanitized artifacts；任一 leak fail closed。
- **Logic**：condition contamination、重试 cherry-pick、缺失分母和 aggregate
  掩盖方向失败会制造虚假结论。用 matrix/config hash、immutable attempt
  history、direction-first report 和 completeness gate 防止。
- **Compatibility**：schema v2 不 silent-accept v1 skeleton/artifact；旧文件
  保留历史，显式转换后才可计数。
- **Performance/Cost**：primary 至少 288 runs，native ablation 还会增加 paired
  runs。支持中断恢复和 tuple filter，但完整 claim 不能以抽样 smoke 替代。
- **Maintenance**：双宿主 CLI 可能漂移。adapter 记录 binary/version/hash，
  未识别版本 fail closed；更新 adapter 时必须提升 policy/version 并重跑。
- **Statistical validity**：12 task clusters/方向仍较小；报告完整 CI 和 task
  分布，CI 含 0 时不作正向 claim。

## 测试计划

- [ ] Offline schema/scanner：

  ```bash
  python3 eval/cross-host/scripts/schema_validate.py --self-test
  python3 eval/cross-host/scripts/scan_artifacts.py --self-test
  python3 eval/cross-host/scripts/run_dry.py
  ```

- [ ] Rust focused tests：

  ```bash
  cargo test eval::host_isolation
  cargo test eval::cross_host
  cargo test eval::coding_bench
  ```

- [ ] CLI dry-run/verify（不得启动宿主）：

  ```bash
  cargo run -- bench cross-host run --root eval/cross-host \
    --runs-per-condition 3 --matrix primary --dry-run \
    --json-out /tmp/remem-cross-host-plan.json
  cargo run -- bench cross-host verify --root eval/cross-host \
    --json-out /tmp/remem-cross-host-verify.json
  ```

- [ ] Public claim negatives：

  ```bash
  python3 scripts/ci/check_public_claims.py --self-test
  python3 eval/claims/claim_gate.py check eval/cross-host/claims-registry.json
  ```

- [ ] Repository gates：

  ```bash
  cargo fmt --check
  cargo check
  cargo clippy --all-targets -- -D warnings
  cargo test
  python3 scripts/ci/check_plugin_version_sync.py
  ```

- [ ] Manual live verification（仅在 SP935-T9 人工授权后）：先做每方向一个
  smoke tuple，核对 auth/sandbox/capture/cleanup/artifact；再执行完整 288
  primary 与 native ablation。smoke 不能进入公开分母。

## 回滚方案

1. 在任何 live run 前，可移除 `bench cross-host` executable surface，并保留
   schema/task/history contract；状态回到 `executable_no_runs` 或
   `infrastructure_only_no_runs`，不得伪造结果。
2. 若 adapter/sandbox 有 leak，立即停止后续 tuple，将已产生 artifact 标记
   invalid 并保留审计，不发布 report；修复必须提升 policy/version 后重跑受
   影响完整矩阵。
3. 若 report/bootstrap/claim gate 有缺陷，撤回 README/CHANGELOG 结果 wording，
   保留原始 immutable artifacts 和旧 report hash，生成新版本报告而不是覆盖。
4. 版本回滚需同步所有 Cargo/plugin/npm/server surface；已提交的失败/泄漏
   evidence 不得因代码回滚删除。

## Human Gates

GH-935 当前 `ready_to_spec` 已由 maintainer 明确授权，write-spec route 的
readiness gate 已满足；本文件仍不构成 `spec_approval`。只有 maintainer
批准 product/tech exact diff、确认宿主 auth/sandbox 与 public-claim security
边界并把 GH-935 置为 `ready_to_implement` 后，才能为当时的 exact
implementation head 收集所需 route evidence 并执行实现门禁。不得把本 packet
里的历史 gate 输出、重复工作搜索或预期路径 manifest 当成未来 head 的 fresh
implement evidence。live smoke、288+ run、最终 claim wording、PR review、
merge 和 release 还需各自独立人工门禁。
