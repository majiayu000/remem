# Product Spec

## Linked Issue

GH-931

complexity: large

Locale: zh-CN

## 当前事实

PR #936 已把 GH-931 的 scaffold 合入 default branch。本 packet 在
`origin/main@5627a74942a41f51bdc03518fce726dbf1b46098` 核对到：

- `eval/coding-bench/conditions.json` 已声明 primary
  `no_memory`、`curated_file_budgeted`、`remem_e2e`，并固定 6-stage /
  12-enum failure taxonomy；
- budgeted curator protocol/schema 与 `eval/claims/registry.json` wording
  gate 已存在，当前三个 claim 均为 `INSUFFICIENT` 且
  `supporting_report: null`；
- Rust `BenchCondition` 和 CLI 仍只接受旧 ID
  `no_memory`、`remem`、`curated_file`；当前 dry-run 是 16 tasks × 3 旧条件
  × 3 runs = 144 个 diagnostic/legacy 计划项；
- `remem_e2e` 仍为 `pending_src_support`，
  `curated_file_budgeted` 仍为 `artifact_schema_only`；没有真正的
  capture→extraction→promotion→retrieval 运行、paired report 或可公开引用的
  flagship outcome。

因此 scaffold/schema 通过不等于产品 claim 已被验证。

## 用户问题

用户需要一个可复验的旗舰证据，回答 remem 在持续变化的 coding project 中，
能否通过真实自动记忆链路，以显著低于人工维护的成本，达到不弱于
target-blind、限时维护的 `MEMORY.md` 的 coding outcome，并且不增加不可接受的
stale/irrelevant memory harm。

当前 near-oracle preload 与 target-aware curated file 只能帮助诊断，不能回答
这个问题。

## 目标

- 把现有 16-task deterministic fixture 接入三个 primary conditions 的真实、
  隔离、可重复运行路径。
- 让 `remem_e2e` 只通过自动 capture、LLM extraction、正常
  review/promotion policy 和 production retrieval 提供记忆。
- 让 `curated_file_budgeted` 遵守 target-blind 人工维护协议并完整计量维护
  成本。
- 生成 task-level paired、condition-order randomized 的 144-run v1
  primary matrix，以及分母、失败阶段、成本、记忆伤害和 citation evidence
  完整的报告。
- 用预注册阈值、task-cluster paired bootstrap、stop-loss 和 report hash
  决定允许公开的 wording。
- live agent/LLM 运行保持显式、限额、可中断恢复；普通 CI 和 dry-run 不得
  隐式发起付费或需要 auth 的调用。

## 非目标

- 不在本 Issue 中扩展到 96 个真实 repo tasks；v1 只完成现有 16-task 基础。
- 不新增 retrieval channel，也不为了 benchmark 调整 production ranking。
- 不把 LLM judge 作为 primary outcome；hidden deterministic score commands
  决定 `resolved_rate`。
- 不继续支持旧 runner ID `remem` / `curated_file` 的兼容 alias；旧 artifact
  只作为明确标注的历史 schema evidence。
- 不把 gold memory、完整 expected evidence、手工 `save_memory` 或 target
  prompt 可见 hidden files 用于 `remem_e2e`。
- 不把 diagnostic conditions 混入 primary denominator 或 public claim。
- 不在本 spec lane 执行 live run、使用宿主 auth、修改 source 或发布结论。

## Behavior Invariants

### 状态、条件与矩阵

1. **B-001** 在三个 primary condition 都可执行、144 个 tuple 完整、report
   验证和 claim gate 完成前，suite 必须显示 `INSUFFICIENT`；缺失数据不得填
   0、PASS 或“已完成”。
2. **B-002** primary condition 闭集必须恰好是
   `no_memory`、`curated_file_budgeted`、`remem_e2e`；旧
   `remem_preloaded`、`curated_file_expert` 与其他 oracle/ablation 只能作为
   diagnostic。
3. **B-003** runner/CLI/machine artifacts 使用新 stable ID；裸
   `remem`、`curated_file` 不得作为 alias 被接受。已有旧 report 必须标注
   legacy schema 并禁止进入新 claim denominator。
4. **B-004** v1 primary plan 必须是
   `16 tasks × 3 conditions × 3 runs = 144` 个唯一 tuple；少一个、重复一个、
   或以失败重试覆盖原 tuple 都不得产生 complete verdict。
5. **B-005** 同一 `(task_id, run_index)` 的三条件必须共享 fixture revision、
   target prompt、hidden score、timeout、sandbox policy、实际执行的 remem/
   agent-runner binary digest，以及分别记录的 target-agent、extraction、
   enrichment、review/promotion、retrieval runtime/profile hashes；condition
   memory surface 是唯一允许差异。
6. **B-006** condition 执行顺序按记录的 seed 随机化，统计单位为 task；
   三次 run 不得被当成三个独立 task 增大显著性。

### Condition 边界

7. **B-007** `no_memory` 必须关闭 remem hooks/MCP/SessionStart、repo memory
   file 和 host-native memory，只暴露当前代码与 target task。
8. **B-008** `remem_e2e` 的 fixture 必须提供 answer-bearing、sanitized、
   schema-valid 原始 session/tool-event payload；历史证据只能从这些 payload
   进入 `captured_events`，再经真实 `extraction_tasks`、自动 extraction、
   review/promotion policy、memories/projections 和 production
   SessionStart/MCP retrieval 到达 agent。gold `memories`/expected answer 只能
   进入 scorer，不得伪装为 captured input。
9. **B-009** `remem_e2e` 必须拒绝直接 DB seed gold memory、手工
   `save_memory`、`render_seeded_remem_context`、完整 gold-evidence preload
   或把 expected answer 写入 prompt。
10. **B-010** extraction/provider 缺失、worker drain 未完成、candidate 未按
    policy 处理或 retrieval evidence 不完整时，该 run 必须明确失败并保留
    stage evidence；不得降级到 preload 或手工补记忆。若 treatment 使用人工
    candidate review/edit/promotion，reviewer 只能看到 closed、hashed、
    gold-free 的 `treatment_review_input_projection`，其中只含 pre-target
    candidate/source provenance/conflict/quality/rubric；必须排除 target
    prompt、gold/expected、hidden/scorer 与 target outcome。全部 review/
    promotion 在 target reveal 前完成并冻结；post-reveal intervention 使 run
    invalid。
11. **B-011** `curated_file_budgeted` 的 curator 只能看到由 schema allowlist
    从历史 `raw_events` 投影出的 chronological `curator_input_projection`；
    projection 必须排除 `expected_memory_facts`、gold `memories`/refs、target
    prompt、hidden score/oracle 与 scorer metadata，并在 curator 启动前
    canonicalize/hash。committed artifact 必须保存该 projection/hash，使
    verifier 可从 fixture 独立重建并证明 gold-free。curator 必须在 target
    task 揭示前完成并冻结 `MEMORY.md`，运行时不得暴露其他 memory surface。
12. **B-012** 每个 budgeted run 必须验证 frozen file hash，并记录人工维护
    minutes、更新/删除/冲突处理次数、字符/token 大小和 budget exceed 状态；
    缺 log、hash 不同或超预算均使该 run 无效。
13. **B-013** `remem_preloaded` 与 `curated_file_expert` 保留为 diagnostic
    upper bounds 时必须使用新名称并明显标记 shortcut；其 outcome 不得被写成
    primary evidence。

### 隔离、失败与恢复

14. **B-014** 每个 condition/run 使用新的 HOME、CODEX_HOME、DB、repo 和 host
    state；不得读取真实用户 HOME、其他 condition/run 的数据或旧 host session。
    target agent 及其 tool subprocess 只能访问 task repo，不得获得 service/
    coordinator 的 auth、DB、artifact、ledger 或 private-root 路径；允许的
    SessionStart/MCP 内容只能经受控 broker 提供。target agent 与 tool
    subprocess 必须 deny-all outbound network（含 DNS、loopback、cloud metadata
    与 public Git host）；模型/provider 流量只能经 service-side protocol broker
    发送，broker 不接受任意 URL/fetch/tool tunneling。task repo 必须是仅含
    approved fixture 的 detached tree，不含 benchmark/gold/hidden files或
    可用 remote URL。
15. **B-015** auth/config 只能通过显式 live-run bootstrap 进入 service-private
    root；stdout/stderr 必须在任何 disk/artifact write 前流式检测与脱敏，
    credential bytes 不得以 raw、ignored、temporary、report 或 git artifact
    形式落盘。检测到 secret 时必须 fail closed、丢弃原 bytes 并记录无 secret
    的 reason code。
16. **B-016** hidden oracle 只在 agent 结束后，于与 agent repo 分离的
    scorer-only clean tree 中 materialize；harness 只能把经校验的 patch 应用到
    该 tree，必须拒绝 symlink/hardlink/device/path collision，并保持 scorer
    bootstrap/import files read-only。agent 读取、修改或影响 hidden content/
    bootstrap 必须 fail closed。
17. **B-017** auth/provider 不可用、capture/extraction/promotion/retrieval
    失败、agent timeout/crash、score/cleanup/scanner 失败都必须形成 typed
    artifact 或显式 suite error，不能静默丢弃。
18. **B-018** 每次尝试有唯一 `attempt_id`；在 target process spawn 前，必须
    把绑定 budget reservation receipt 的 `target_started` transition 以 CAS
    append 到同一 anchored authoritative remote ledger；remote commit durable
    后才可 spawn。重试保留此前失败 artifact；
    recovery 发现 started 但无 terminal artifact 时，必须一次性生成 immutable
    `abandoned_after_target_start` failure（`resolved=0`）并封闭该 run index，
    不得重试或永久留作“缺失”。target 已启动后的其他 outcome failure 同样
    留在预注册分母，不允许挑成功重跑。
19. **B-019** resume 只补缺失 tuple；duplicate tuple、hash drift、partial
    artifact 或已完成 artifact overwrite 必须被拒绝。
20. **B-020** dry-run、schema validation、report verify 和普通 CI 不读取
    provider key、不启动 agent、不访问网络；live run 必须引用 default branch
    上经 maintainer review/merge 的 immutable approval policy。其 stable
    `approval_key` 只能由 pre-merge 可知且不包含自身的 canonical policy
    digest、approval PR number 与 repository identity 派生；不得包含承载该
    key 的 Git blob/tree/commit OID。merge/review/head-tree attestation 由
    verifier 对 registry blob 另行查询，不能写入 key preimage。approval 必须绑定
    exact executable/profile/fixture/registration hashes、允许 tuple、累计
    calls/cost 上限，以及 immutable canonical pricing snapshot：币种固定为
    USD、provider/model SKU、effective timestamp、input/output/cache/tool
    token 单价、各 call-kind 最大 input/output/cache/tool tokens 与明确的向上
    取整规则。调用方不得提交 `worst_case_cost`；service broker 必须从 reviewed
    rates × per-call ceilings 保守计算 reservation，计算溢出、未知 SKU、价格
    漂移或 currency/rounding mismatch 均在调用前 fail closed。每次 billable
    call 前必须在 authoritative shared ledger durably reserve该计算值，
    crash/abandoned reservation 仍按上限计费。
    ledger 必须从 policy 中固定的 genesis OID 延伸，且 authority phase fresh
    验证保护该 ref 的 active non-bypassable ruleset：禁止 delete/force push、
    bypass actor 为空并覆盖管理员与 automation；任一保护/audit drift 都
    fail closed。resume、换 clone/`execution_id`、并发或拆单都不得重复领取预算。

### Attribution、失败分解与 claim

21. **B-021** 每个 `remem_e2e` run 必须记录 captured event、extraction task、
    candidate/review/promotion、memory/projection、selected/injected、cited/used
    refs；ref 必须属于同一 run/project 且能回溯。
22. **B-022** 每个 memory failure 必须按
    Capture→Extraction→Consolidation→Retrieval→Context compilation→Reader/use
    的 earliest-proven-causal-stage 顺序选择恰好一个 root stage/code，并可另列
    downstream consequences；无法证明 root 时输出明确 `unclassified` suite
    error，不能按实现顺序猜测或把多个 consequence 当多个 root。
23. **B-023** report 必须公开每个 condition/task 的成功、失败、缺失分母，
    `resolved_rate`、compile/timeout/wrong-file、tokens、wall time、人工维护
    成本、memory helped/hurt、stale/irrelevant/missing 和 citation 指标；无数据
    用 `null` + missing count。144 个 primary tuple 的 scanner-passed sanitized
    run records 与 source manifest 必须作为 committed release evidence 保留，
    并足以独立重算分母、失败、成本、attribution、report hash 和 gate input；
    `/tmp` 或 aggregate-only report 不构成可复验证据。
24. **B-024** 每个 task/condition 的 outcome 固定为三次预注册 run 的二元
    `resolved` 均值；target-started timeout/crash/score failure 计 0，pre-target
    缺失或 integrity-invalid tuple 使 matrix `INSUFFICIENT`、不得插补。bootstrap
    每次以 16 个 task cluster 有放回抽样，并在每个抽中 task 上重算 treatment
    与 control 三-run 均值差；使用固定 seed/算法的 percentile 95% CI，报告
    absolute pp difference。pair/hash 缺失时 verdict 为 `INSUFFICIENT`。
25. **B-025** `remem_e2e` vs `no_memory` 的正向 claim 需要 resolved rate
    提升至少 10pp 且 95% CI 下界 > 0。
26. **B-026** `remem_e2e` vs `curated_file_budgeted` 的 claim 需要预注册
    margin 恰为 3pp、treatment-minus-control 的 paired 95% CI 下界
    `>= -3pp`，且同一 task/session denominator 上的总人工维护时间下降至少
    70%。`remem_e2e` 的 manual candidate review/promotion minutes 必须计入
    treatment；缺 treatment log 或 control 总分钟为 0 时 verdict 为
    `INSUFFICIENT`。
27. **B-027** stop-loss 只对 48 个预注册 `remem_e2e` primary tuples 计算。
    `memory_hurt` numerator 是 paired `no_memory` resolved=1、`remem_e2e`
    resolved=0 且 attribution 证明 injected/cited/used memory 导致错误动作的
    tuple；`stale_memory_followed` numerator 是 agent cited/used stale/
    superseded item 并据此采取错误动作的 tuple；两者 denominator 都是 48。
    任一 tuple 的 required attribution 缺失使 gate `INSUFFICIENT`，不得从分母
    删除或填 0；任一 rate 超过 2%/1% 优先使正向 claim FAIL。
28. **B-028** registry 的 immutable `registration_projection` 必须在任何使用
    official 16-task fixture 的 live smoke/official run 前锁定 dataset、所有
    executable/profile hashes、timeout、runs、metric、failure/missing rules、
    exclusions、bootstrap seed/algorithm、threshold 和 wording templates。
    run artifact 只绑定该 projection digest；后续 mutable
    `result_bindings`（status/report hash/approved wording）不得改变该 digest。
    若要在看到 live outcome 后改 projection，必须创建新的 benchmark version
    和 disjoint fixture，旧 runs 永久不得进入新 claim。
29. **B-029** public wording 必须绑定 committed report hash、gate verdict、
    claim ID 与独立 maintainer-approved exact UTF-8 wording。非 PASS 时只能使用
    预注册 directional/insufficient wording；PASS 时 CI 也必须逐条要求 public
    text 与 `result_bindings.allowed_wording` 精确匹配并携带对应 report link，
    不得仅因存在某个 PASS 就放行任意幅度或范围的 superiority claim。
30. **B-030** readiness、spec approval、live-run auth/cost、security、
    final review、merge、public wording 和 release 均保持独立人工门禁。

## 验收标准

- [ ] Rust/CLI 只使用新 condition ID，旧 reports 被明确隔离为 legacy。
- [ ] 16-task × 3 primary × 3 runs 的 offline plan 精确为 144，dry-run 零
  agent/network/provider access。
- [ ] `remem_e2e` 真实自动链路通过正向与禁止 shortcut 的负向测试。
- [ ] `curated_file_budgeted` protocol、freeze hash、人工成本和超预算负例通过。
- [ ] condition/run 隔离、timeout、cleanup、resume、attempt integrity 和
  hidden-test/deny-network 边界有 deterministic tests。
- [ ] 6-stage / 12-enum failure attribution 与完整 source-to-use refs 可验证。
- [ ] 144-run official artifacts 有 committed sanitized run-record bundle 与
  source manifest；paired report、成本、attribution 与 stop-loss 可独立复算。
- [ ] claim registry 在 official run 前锁定，wording 只引用 hash-bound verdict。
- [ ] 没有 official runs 时保持 `INSUFFICIENT`，不产生 public outcome claim。

## Boundary Checklist

| Boundary | Verdict |
| --- | --- |
| Empty / missing input | covered: B-001, B-004, B-010, B-012, B-017, B-023, B-024 |
| Error / provider / auth | covered: B-010, B-015, B-017, B-020 |
| Concurrency / ordering | covered: B-005, B-006, B-011, B-014, B-019 |
| Retry / idempotency | covered: B-018, B-019 |
| Compatibility | covered: B-003, B-013 |
| Security / privacy | covered: B-014, B-015, B-016 |
| Silent degradation | covered: B-001, B-010, B-017, B-022, B-023 |
| Evidence integrity | covered: B-004-B-006, B-021-B-029 |
| Human gates | covered: B-020, B-028-B-030 |

## 边界情况

- provider key 缺失时，offline plan 仍可验证；live `remem_e2e` 明确失败且不
  改跑 `remem_preloaded`。
- curator 超时或 target 提前泄漏时，budgeted artifact 无效且不能用 expert
  file 替代。
- 一个 task 三次都失败时仍保留全部 outcome；不得从分母删除。
- matrix 已执行但 claim registry 未锁定时，只能产生 invalid/insufficient
  report，不得事后锁阈值。
- 旧 baseline report 保持历史可读，但不能因为字段名称相近而进入 v1 flagship
  report。

## 发布说明

先交付可执行 runner、artifact 和 offline gates；再由 maintainer 单独批准
限额 live run。只有 144-run report、paired bootstrap、maintenance cost、
stop-loss 和 wording gate 全部通过后，README/release surface 才能引用经批准
的具体结论。
