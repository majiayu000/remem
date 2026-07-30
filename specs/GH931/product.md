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
   `16 tasks × 3 conditions × 3 runs = 144` 个唯一 tuple；official canonical
   matrix key 固定为
   `(matrix_namespace=issue385-v1/official-v1, task_id, condition, run_index)`。
   少一个、重复一个、或以失败重试覆盖原 tuple 都不得产生 complete verdict。
   live smoke 必须由 reviewed approval policy 分配
   `run_phase=smoke` 与不属于 `issue385-v1/official-v1` 的独立
   `matrix_namespace`；caller 不能选择/覆盖 namespace，smoke key、attempt 和
   artifact 永久不能转成 official evidence。smoke 与 official 仍共享同一个
   authoritative cumulative-budget ledger，不得用新 ledger/ref 绕过总上限。
5. **B-005** 同一 `(task_id, run_index)` 的三条件必须共享 fixture revision、
   target prompt、hidden score、timeout、sandbox policy、approval-pinned
   OS/security-owner-anchored host supervisor 及实际执行的 remem/agent-runner
   binary digest，以及分别记录的
   target-agent、extraction、
   enrichment、review/promotion、retrieval runtime/profile hashes；condition
   memory surface 是唯一允许差异。registration 还必须固定一个 UTC
   `evaluation_as_of` 与 virtual-clock policy；所有 paired condition 的
   capture/candidate/promotion timestamps、TTL/active-memory decisions、
   SessionStart、PromptSubmit、MCP search/get-detail、memory expiry、temporal
   parsing/fact validity、age/staleness、usage、graph、rerank、audit/explain 与
   access feedback 必须显式使用同一个 evaluation clock。benchmark MCP
   tool set 闭集只能含 `search`/`get_observations`，其他 read/write/raw/
   workstream tools 必须不可见。能影响 target-visible
   selection/content/order 的 Rust wall clock 或 SQLite `now` 读取、clock drift
   都使 pair 无效。approval expiry、budget、timeout 与 supervisor duration
   必须使用独立的真实 security/operational clock，不能被 virtual clock 延长或
   回退。
6. **B-006** condition 执行顺序必须由 live outcome 前锁定在
   `registration_projection` 的 condition-order seed、PRNG algorithm/version
   和完整 canonical tuple permutation 派生；planner 重算的顺序/digest 必须
   byte-identical，不能在 plan/run 后才记录 seed。统计单位为 task；三次 run
   不得被当成三个独立 task 增大显著性。

### Condition 边界

7. **B-007** `no_memory` 必须关闭 remem hooks/MCP/SessionStart、repo memory
   file 和 host-native memory，只暴露当前代码与 target task。
8. **B-008** `remem_e2e` 的 fixture 必须提供 answer-bearing、sanitized、
   schema-valid 原始 session/tool-event payload；历史证据只能从这些 payload
   进入 `captured_events`，再经真实 `extraction_tasks`、自动 extraction、
   review/promotion policy、memories/projections 和 production
   SessionStart/MCP retrieval 到达 agent。gold `memories`/expected answer 只能
   进入 scorer，不得伪装为 captured input。adapter 必须按注册的
   `history_episodes`/`raw_events` 原始嵌套数组顺序 flatten，并派生全 projection
   连续 `source_ordinal=0..N-1`；ordinal 是唯一 canonical order，`event_id`
   只作身份、不得排序或破同秒平局。timestamp 按 ordinal 非递减且允许同秒；
   ordinal 必须进入 call content，每个 call index 与 inserted row ID 随 ordinal
   严格递增。gap/duplicate/shuffle、timestamp 回退、event-ID sort 或 call/row
   inversion 在首个 commit 前失败。
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
    promotion 必须按 task 跨三个 run index 批量完成并冻结，且在该 task 任一
    target reveal/outcome 前完成；否则必须证明每个后续 reviewer/curator 的
    identity 与不可篡改 assignment 从未接触该 task 的 target/outcome。
    post-reveal intervention 或已暴露 actor 的后续 preparation 使 run invalid。
11. **B-011** `curated_file_budgeted` 的 curator 只能看到由 schema allowlist
    从历史 `raw_events` 投影出的 chronological `curator_input_projection`；
    projection 必须排除 `expected_memory_facts`、gold `memories`/refs、target
    prompt、hidden score/oracle 与 scorer metadata，并在 curator 启动前
    canonicalize/hash。committed artifact 必须保存该 projection/hash，使
    verifier 可从 fixture 独立重建并证明 gold-free。同一 task 的三个 run
    projection/output 必须在任何 target task 揭示前由未暴露 curator 完成并
    冻结 `MEMORY.md`；运行时不得暴露其他 memory surface。
12. **B-012** 每个 budgeted run 必须验证 frozen file hash，并记录人工维护
    minutes、更新/删除/冲突处理次数、字符/token 大小和 budget exceed 状态；
    人工 minutes 只能来自 trusted supervisor 对 exact interaction/projection/
    frozen output 记录的 monotonic start/end receipt，不能接受 curator/reviewer
    自报 elapsed。committed release evidence 必须保存 sanitized frozen
    `MEMORY.md` exact bytes 的 content-addressed record，使 verifier 可重算 hash
    并检查实际 control surface；缺 log/receipt/bytes、hash 不同或超预算均使
    该 run 无效。
13. **B-013** `remem_preloaded` 与 `curated_file_expert` 保留为 diagnostic
    upper bounds 时必须使用新名称并明显标记 shortcut；其 outcome 不得被写成
    primary evidence。

### 隔离、失败与恢复

14. **B-014** 每个 condition/run 使用新的 HOME、CODEX_HOME、DB、repo 和 host
    state；不得读取真实用户 HOME、其他 condition/run 的数据或旧 host session。
    target agent 及其 tool subprocess 只能访问 task repo，不得获得 service/
    coordinator 的 auth、DB、artifact、ledger 或 private-root 路径；允许的
    SessionStart/MCP 内容只能经受控 broker 提供。target namespace 必须拒绝
    DNS、public/RFC1918/metadata 与 Unix socket；仅 pinned Codex 主进程可连接
    namespace 内无外网的 private-loopback provider adapter，adapter 再通过
    supervisor 预建的 bounded pipe 访问 mount 外 broker。tool subprocess 连
    loopback 也必须被 OS policy 拒绝；平台无法证明该区分时禁止 live run。
    broker 不接受任意 URL/fetch/tool tunneling。task repo 必须是仅含 approved fixture
    的 detached tree，不含 benchmark/gold/hidden files 或可用 remote URL。
15. **B-015** auth/config 只能通过显式 live-run bootstrap 进入 service-private
    root；stdout/stderr 必须在任何 disk/artifact write 前流式检测与脱敏，
    credential bytes 不得以 raw、ignored、temporary、report 或 git artifact
    形式落盘。检测到 secret 时必须 fail closed、丢弃原 bytes 并记录无 secret
    的 reason code。
16. **B-016** hidden oracle 只在 agent 结束后，于与 agent repo 分离的
    scorer-only clean tree 中 materialize；scorer 使用独立 OS principal/process/
    tree，controller 永不 import/exec patched code。无 hidden mount 的不可信 code
    worker 只能经 bounded、closed-schema RFC 8785 JCS JSON RPC 与 scorer 边界交互；
    scorer 拒绝 symlink/hardlink/device/path collision 并保持 oracle/bootstrap
    read-only。stdout、exit 0、visible tests、worker 自报结果都不能定 PASS；
    monkeypatch/shared interpreter、异常或 malformed RPC 必须 fail closed。
17. **B-017** auth/provider 不可用、capture/extraction/promotion/retrieval
    失败、agent timeout/crash、score/cleanup/scanner 失败都必须形成 typed
    artifact 或显式 suite error，不能静默丢弃。
18. **B-018** 每次尝试有唯一 `attempt_id`；任何 curator/treatment reviewer
    interaction、billable preparation 或 target work 开始前，trusted supervisor
    必须先把绑定包含 `run_phase`/`matrix_namespace` 的 canonical matrix key、
    projection hash、budget reservation 与 timing policy 的
    `pre_target_work_started` transition 以 CAS append 到 anchored authoritative
    remote ledger。human interaction start/end、frozen output
    digest 与消耗由 supervisor 继续 append；abandon/crash 必须封为
    `abandoned_before_target`、保留或按批准上限保守计入人工成本并封闭该
    run index，不能从 fresh clone 重做。target process spawn 前还必须 durable
    append `target_started`。重试保留此前失败 artifact；
    recovery 发现 started 但无 terminal artifact 时，必须一次性生成 immutable
    `abandoned_after_target_start` failure（`resolved=0`）并封闭该 run index，
    不得重试或永久留作“缺失”。每个 terminal outcome 先冻结 receipt-free、
    immutable RFC 8785 JCS payload；payload 不含 terminal attestation/checkpoint、
    source-manifest/report hash 或任何由自身 digest 派生的字段。supervisor
    先计算 payload digest 并将 matrix key、cost/timing/frozen-surface digests CAS
    seal 到同一 ledger；receipt 产生后，source manifest 才以 detached mapping
    绑定 payload digest、terminal attestation 与 checkpoint receipt。verifier
    依次验证 payload→ledger seal/signature/ancestry→checkpoint→mapping；没有
    匹配链的 artifact/manifest 不可信。target 已启动后的其他 outcome failure同样留在预注册
    分母，不允许挑成功重跑。
19. **B-019** resume 只补缺失 tuple；duplicate tuple、hash drift、partial
    artifact 或已完成 artifact overwrite 必须被拒绝。
20. **B-020** dry-run、schema validation、report verify 和普通 CI 不读取
    provider key、不启动 agent、不访问网络；默认 report 只验证 execution-time
    bundle receipts/proofs，明确不声称当前 authority freshness。live run 必须引用 default branch
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
    call 前，broker 必须以实际序列化 request 验证 input/cache/tool token
    counts 不超过 reservation，并通过 provider 强制参数设置 output/reasoning/
    tool/cache ceilings；provider/API 不能硬性执行或 broker 无法在超限前终止时
    禁止 dispatch。随后在 authoritative shared ledger durably reserve 该计算值，
    crash/abandoned reservation 仍按上限计费。
    live approval verify、smoke、official run 与显式 network freshness audit
    都必须由固定、
    root-owned immutable host supervisor 从 authority 取得 expected digest 并执行
    `openat(O_NOFOLLOW)`、same-fd hash/fstat 和 same-handle exec；runner 以同一
    primitive 启动 agent，artifact 验证 OS/security-owner-key attestation。
    caller path/digest、一次 path hash或“先验后开”不构成验证。
    每个 reservation 前，隔离 authority broker 必须 fresh 重验 approval 未过期、
    merge/review 仍有效、ledger 从 genesis 的 authenticated ancestry、Sigstore
    Rekor public-good transparency log 的 latest verified bundle chain，以及
    保护 exact ref 的两个 active GitHub rulesets。update-authority ruleset 只启用
    `Restrict updates`，并只把 approval-pinned ledger-writer GitHub App 列为
    唯一 bypass/update actor；用户、admins、Actions 与其他 apps 都不得更新。
    integrity ruleset 的 bypass actor 列表必须为空，并对所有 actor 独立执行
    restrict deletion、block force push、require signed commits。两个 ruleset
    的 ID、canonical hash、target ref 和 active state 都进入 approval；writer
    App 对前者的 bypass 不适用于后者，也不能替代 ancestry/signature/Rekor
    验证。任一 approval/protection/log/history drift 都 fail closed。
    authoritative ledger 的每个 transition、reservation 和 terminal seal 只能
    由 dedicated ledger-writer broker workload identity append；trusted
    supervisor / isolated authority broker 的 originator role attestation 必须
    进入 canonical payload。append envelope 必须由 writer 以 approval-pinned
    key 做 cryptographic signature，覆盖 previous ledger head、monotonic
    sequence、canonical payload、`approval_key`、`execution_id` 和 originator
    role。remote CAS 成功后，writer 必须把只含 repo/ref/sequence/tip/
    ledger digest/previous Rekor bundle digest 的 signed DSSE checkpoint 提交到
    Sigstore Rekor public-good transparency log；active shard URL、operator/log
    identity 和验证 key 只能从 approval-pinned TUF `TrustedRoot`/
    `SigningConfig` 解析，禁止 hard-code rotating endpoint。Rekor inclusion
    proof、signed checkpoint、consistency proof 与严格递增 log index 验证并
    durable 保存前，不得接受 transition 或 dispatch。verifier 在每次状态迁移
    和显式 network freshness audit 时都重验 writer signature chain、Rekor
    bundle/consistency chain、
    TUF trust/key rotation、identity/role permission 与两个 exact rulesets。
    匿名/普通 Git credential、未授权/已撤销 identity、signature/bundle 缺失或
    不匹配、相对 pinned/previous checkpoint 的 rollback/consistency failure、
    writer allowlist/rules drift 一律 fail closed。Rekor 是相对 GitHub/writer
    的 external operator anchor；无独立 witness/gossip 时不能声称检测一个恶意
    log operator 为本客户端持续提供的自洽 split view，该威胁若进入 live
    approval 必须先另行批准 witness quorum。resume、换 clone/`execution_id`、
    并发或拆单都不得重复领取预算。network freshness audit 必须在不改 report
    bytes 的前提下输出 authority-signed detached receipt，绑定
    `report_sha256`、ledger tip、ruleset/TUF/Rekor digests、`observed_at` 与
    `expires_at`；publication/closure/release 要求 exact-report receipt 未过期。
    network denial、stale receipt、wrong-report binding 或远端 drift 均 fail closed。

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
    每个 receipt-free record 的 JCS digest 必须经 source manifest detached mapping
    匹配 authoritative ledger 中 supervisor-sealed terminal matrix-key/artifact
    digest 及 checkpoint receipt；control 必须解析到 committed
    content-addressed sanitized frozen bytes。`/tmp`、self-rehashed mutable tree
    或 aggregate-only report 不构成可复验证据。
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
    `registration_projection` 必须为每个 task 预注册并 hash 一个 scorer-only
    `memory_harm_rules` 闭集；每条规则绑定 source provenance fact/content hash、
    canonical cited/used event predicate、严格 happens-before 关系、normalized
    tool action/patch/scorer failure fingerprint，以及在 `evaluation_as_of` 时
    是否 stale/superseded。规则必须预注册互斥的 `memory_caused`、
    `independent_cause`、`no_wrong_action` 分类、deterministic evaluation order
    与 verifier algorithm/version；`no_wrong_action` 只能由完整 sealed trace
    证明没有预注册 wrong-action predicate 命中，不能从缺日志推断。target agent
    不得看到规则内容。
    verifier 对每个 `remem_e2e` tuple 只能消费 ledger-sealed event/patch/scorer
    evidence 并必须得到恰好一个 terminal classification。`memory_hurt`
    numerator 是 paired `no_memory` resolved=1、`remem_e2e` resolved=0 且
    classification=`memory_caused` 的 tuple；`stale_memory_followed` numerator
    是 classification=`memory_caused` 且唯一规则标记 stale/superseded 的 tuple，
    不以 paired `no_memory` outcome 为前提。`independent_cause` 或
    `no_wrong_action` 可机械记为相应 numerator false。零匹配、多匹配、
    evidence/hash/trace 缺失或无法唯一分类时必须标记
    `ambiguous_causality` 并使 gate `INSUFFICIENT`，不得记作 false、从分母删除
    或填 0。两者 denominator 都是 48，任一 rate 超过 2%/1% 优先使正向 claim
    FAIL。
28. **B-028** registry 的 immutable `registration_projection` 必须在任何使用
    official 16-task fixture 的 live smoke/official run 前锁定 dataset、所有
    executable/profile hashes、timeout、runs、metric、failure/missing rules、
    exclusions、official `run_phase`/`matrix_namespace`、smoke namespace
    non-collision policy、`evaluation_as_of`/virtual-clock policy、
    condition-order seed/PRNG version/完整 tuple permutation、bootstrap
    seed/algorithm、threshold 和 wording templates。它只能在
    CLI/docs/version-sync 完成并从 exact final
    implementation head reproducibly build、记录最终 remem/agent binary hashes
    后冻结；实现期 synthetic registry 不能冒充 final registration。
    run artifact 只绑定该 projection digest；后续 mutable
    `result_bindings`（status/report hash/approved wording）不得改变该 digest。
    若要在看到 live outcome 后改 projection，必须创建新的 benchmark version
    和 disjoint fixture，旧 runs 永久不得进入新 claim。
29. **B-029** public wording 必须绑定 committed report hash、gate verdict、
    claim ID 与独立 maintainer-approved exact UTF-8 wording。非 PASS 时只能使用
    预注册 directional/insufficient wording；PASS 时 CI 也必须逐条要求 public
    text 与 `result_bindings.allowed_wording` 精确匹配并携带对应 report link，
    不得仅因存在某个 PASS 就放行任意幅度或范围的 superiority claim。
30. **B-030** current-contract approval、live-run auth/cost、security、final
    review、merge、public wording 和 release 均保持独立人工门禁。

## 验收标准

- [ ] Rust/CLI 只使用新 condition ID，旧 reports 被明确隔离为 legacy。
- [ ] 16-task × 3 primary × 3 runs 的 offline plan 精确为 144，dry-run 零
  agent/network/provider access。
- [ ] smoke/official `matrix_namespace` 纳入 canonical key、approval、artifact
  与 ledger record；smoke 永不封闭 official key，二者仍共享同一累计预算 ledger。
- [ ] `remem_e2e` 真实自动链路通过正向与禁止 shortcut 的负向测试。
- [ ] `curated_file_budgeted` protocol、freeze hash、人工成本和超预算负例通过。
- [ ] 同 task 三 repetitions 的 control/treatment preparation 在任一 target
  reveal 前 batch-freeze；人工成本由 supervisor monotonic receipts 产生，
  frozen control exact bytes 可从 committed content-addressed evidence 重算。
- [ ] condition/run 隔离、timeout、cleanup、resume、attempt integrity 和
  hidden-test/deny-network 边界有 deterministic tests。
- [ ] pre-target work、target start 与 terminal artifact digest 都由 authoritative
  ledger CAS seal；每次 append 与 report 都验证 dedicated writer signature、
  originator role、TUF/Rekor bundle chain 与两个 exact rulesets；每个 billable
  dispatch 前重验 authority/rulesets/TUF/Rekor/ancestry 并硬性执行 input/
  output/reasoning/cache/tool token ceilings。
- [ ] 6-stage / 12-enum failure attribution 与完整 source-to-use refs 可验证。
- [ ] 144-run official artifacts 有 committed sanitized run-record bundle 与
  source manifest；paired report、成本、attribution 与 stop-loss 可独立复算。
- [ ] `memory_hurt` / `stale_memory_followed` 只由预注册的 closed causal rules
  对 sealed evidence 机械分类；任一 paired regression 的缺失、多义或未覆盖
  因果证据使 verdict 为 `INSUFFICIENT`。
- [ ] claim registry 在 official run 前锁定，wording 只引用 hash-bound verdict。
- [ ] registration 在 final binary/version 完成后锁定 executable hashes、
  `evaluation_as_of`、PRNG/version 与完整 condition permutation；clock/order
  drift 的 pair 被拒绝。
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
