# Product Spec：CurrentTruth 统一读取契约

## Linked Issue

GH-933

PR #939 已于 2026-07-26 合并（merge commit `0ed42e3d`），仅交付 Phase A
只读 projection baseline，并以 `Refs #933` 保持 GH-933 开放。PR #965
退役旧的仓库内执行工作流后，本文件只保留 issue-level 规划证据；规范性当前契约在
`docs/specs/GH933/`。本次修订定义剩余 Phase A hardening，不表示 Phase B、
Phase C 或 GH-933 整体完成。

## 用户问题

remem 已经保存 evidence、memory、observation、user-context claim、relation
等多种知识对象，但它们使用不同的状态与时间语义。调用方目前难以稳定回答：
在指定时间和 scope 内，系统认为哪些 claim 当前成立、哪些互相冲突、为什么
选择某个结果，以及每个结论由什么 evidence 支持。

如果每个调用方自行解释 `stale`、`suppressed`、`expired`、`archived`、
`compressed`、`superseded` 等状态，同一份数据可能产生不同答案；无数据、
证据不足或冲突也可能被错误地包装成确定事实。

## 目标

- 提供 versioned `EvidenceView`、`ClaimView`、`RelationView` 和
  `CurrentTruthView` 读取模型，让调用方消费同一套 truth 语义。
- 将 publication、validity、retention 与 visibility 分开表达，避免把存储、
  策略可见性或发布状态误当作事实真假。
- 对 scope、`as_of`、supersedes、evidence trust、冲突和证据不足采用可解释、
  确定性的选择规则。
- Phase A baseline 先提供可重建的只读 projection；下一份 Phase A hardening
  follow-up 补齐 scope/relation 隔离、可诊断 provenance、只读证明、bounded
  lookup，以及 exact history 所需的窄 route/lifecycle ledger migration/backfill 和
  route/lifecycle writer instrumentation；Phase B 再消费 projection，Phase C
  评估更广泛的 Claim writer 收敛。
- 在所有阶段保留 canonical ref、evidence refs 和明确的
  `TruthSelectionReason`，使结果可以审计。

## 非目标

- 不替换 canonical memory/claim 表；只新增 route/lifecycle history 所需的
  additive ledger/index，并对可证明旧历史 backfill，不能证明的范围显式 fail closed。
- 不引入新的图数据库或实时 multi-agent blackboard。
- 不让 LLM 任意裁决 truth，也不把模型自报 confidence 当作真实性分数。
- 不把所有 event 或 generated enrichment 自动提升为 canonical Claim。
- Phase A hardening follow-up 不接入 Context Bundle，不交付 worktree/task
  selector。除 route/lifecycle ledgers、对应 migration/backfill/writer 原子
  instrumentation 和 duplicate capture timestamp immutability 外，不做一般
  writer convergence；它不声明 Phase B/C 完成。
- 本规格不把 archived、compressed 或 suppressed 简化为 false。

## Behavior Invariants

1. CT-001 projection 必须返回带版本的 `EvidenceView`、`ClaimView`、
   `RelationView` 和 `CurrentTruthView`；同一版本对同一可见输入和同一查询
   必须产生确定性相同的结果。Phase A hardening 的 v2 subject identity 必须
   显式区分 source、canonical owner、memory scope、kind 与 key；memory subject
   由 `(ClaimSource::Memory, owner_scope, owner_key, normalized scope,
   memory_type, nonempty-topic-key-or-singleton)` 构成。`topic_key` 为 `NULL`
   或 exact-empty 时必须使用 `memory:<id>` singleton；所有 nonempty key（包括
   纯空白）按 bytes 原样形成 slot。不能让不同 owner、
   global/workspace/project scope、`memory_type` 或两个 singleton-key row 互相竞争。
   user-context
   subject 也必须携带其 exact `owner_scope`/`owner_key`，memory-only scope
   dimension 对它为 `None`。
2. CT-002 lifecycle 必须分别表达 publication、validity、retention，并将
   visibility/policy suppression 独立表达；`Archived` 不等于无效，
   `Compressed` 不等于 false，`Suppressed` 不得自动改写 claim 的真假。
3. CT-003 查询必须先应用 scope 隔离。Phase A 至少支持 project 和 branch；
   project 不匹配的 claim 不得泄露。`branch=Some(B)` 明确表示 branch-scoped
   查询，只能看到 branch-neutral 与 exact `B` rows；`branch=None` 明确表示
   branch-agnostic 查询，可以看到该 project 的全部 branch rows，而不是
   branch-neutral-only 的隐式默认。Project memory inclusion 必须复用 canonical
   repo owner/target predicate，且 full/legacy 两个 arm 都必须排除 normalized
   global；owner-null placement 仅是 atomic legacy fallback。scope cleanup 后的
   stale placement、`source_project` 或 non-repo
   target 不得让 tool/domain/workspace/workstream/session/user-owned memory 泄入
   Project；repo owner/target/placement 不同可以是合法 routing。Owner query
   必须读取 canonical owner exact-match 的 memories 与 user-context claims，
   包括 Owner-scoped global/legacy rows，并保持 all-branch semantics；
   user-claim compatibility wrapper 仍只返回 UserContextClaim，只 bounded 读取
   selected claims 可应用的 `user_claim`/`pattern` suppression 与显式引用 memory，
   不枚举 memory-only suppression。explicit historical query 必须先从持久、
   scope-indexed `memory_route_ledger` 发现候选，再从完整 state-version chain
   恢复 cutoff 时的 owner/target/scope、`memory_type` 与原始 nullable
   `topic_key`；ledger 保留 NULL/空字符串区别，映射时两者仍按 singleton 规则。
   A→B→C 的 B 必须可发现，即使 creation/current 都不是 B。六类 production INSERT
   在写前取得稳定 request ID 后由 trigger 建 v1；normal save upsert、Markdown
   existing-row import 与 scope cleanup 三类 UPDATE 统一走
   canonical route-transition service，在实际
   placement/branch/scope/source/target/owner/memory-type/topic-key/topic-domain/
   routing/context tuple 变化时同 transaction
   append route version；normal save 的 target selector 总要求同 `memory_type`，
   所以只验收同 type 的 raw-key transition；type transition 仅允许 Markdown
   stable-`source_id` path。Markdown 使用 `source_kind=markdown_import`，同值
   assignment 合法；scope cleanup 还 append same-status lifecycle version 与
   audit mirror，其他 bypass 由 guard 拒绝。
   migration 可复制 surviving validated evidence，但旧 save/Markdown mutation
   没有 exhaustive durable log，30-day events 也可能已删；只有 exhaustive proof
   才标 complete，否则只从 migration epoch forward-only，任何更早查询都在
   scope filtering 前返回 `unreconstructable_routing_history`。Project/Owner membership 与
   SubjectIdentity 使用包含 scope 的 route-at-t，equality 使用 new route。
   validated Markdown project→global 在 cutoff 前属于旧 Project，equality/之后
   属于新 Owner。只有 missing/discontinuous/contradictory/invalid-scope 或
   legacy/forward-only coverage gap fail closed；合法 scope transition 不报错。
   relation 的两个
   endpoint 也必须同时属于该查询的 scoped claim set。`Supersedes` 只有两端属于同一完整 subject
   identity 才能改变 winner；普通 `Refutes` 同样只在同一 identity 内裁决。
   唯一 cross-identity decision exception 是 canonical writer 记录且可验证的
   同 owner、同 normalized scope、同 normalized branch
   (`COALESCE(branch, '')` exact equal) 的 preference conflict：
   它可以连接两个不同 topic keys，但只能在各 identity 内部 resolution 完成后
   将仍 surviving 的两个 endpoint 都标为 `Contradicted`，不得合并 identity，
   也不得参与 supersedes、trust 或 recency。scope 外 relation 不得改变 scope
   内结果。
   `Supports`/`DerivedFrom` 等 provenance-only relation 可以连接同一 scope
   内的不同 typed subject，但只能作为 winner 的 provenance 输出，不能参与
   survivor、trust 或 recency 决策。`memory_edges.edge_type` 的 closed writer
   domain 只有 `supersedes`、`duplicates`、`conflicts`、`derived_from`、
   `merged_into`、`split_from`，分别映射 Supersedes(new→old)、
   Supports(from→to)、Refutes(from→to) 与 DerivedFrom(to→from)。任何 scoped
   unknown/newer/typo kind 返回 table/edge/raw-value contextual error。
   worktree/task selector 属于 GH-933
   后续阶段，不是 Phase A hardening 的完成项。compatible selector 没有 row 时
   返回 `truths=[]`；只有已加载 identity 无 survivor 才返回 `Unknown`。
4. CT-004 指定 `as_of` 时，projection 只能使用该时点已存在且在有效时间窗内
   的 claim、relation 和 evidence。memory 的
   `effective_memory_knowledge_epoch` 统一用于 ClaimView、memory SourceRef 与
   SourceTrustClass：canonical result operation 必须匹配 operation epoch 的
   route/identity state，不要求 later current identity 仍相同；再纳入 candidate
   completion 与 validated memory/candidate acknowledgement 取 max。仅
   `updated_at_epoch` 不能证明 ingestion。无
   proof 的 memory 只可用于 `as_of=None` current snapshot，explicit historical
   必须排除/`Unknown`；这也定义了无 operation-log 的 canonical procedure
   memory。candidate/result 只匹配 completion 的 initial owner/project/scope/
   type/raw key route state 一次；后续 candidate-linked owner/project/scope/type/
   key change 必须由按 `(effective_at_epoch,id)` 排序、version/previous link 连续
   且 terminal=current 的 route ledger 证明。cutoff 折叠所有 epoch `<=as_of`
   的 state 以计算 membership 与 emitted `SubjectIdentity`，不再拿 immutable
   candidate identity 对 cutoff/today state 做 exact-match；缺链/coverage/terminal gap 返回 routing-
   history error，其他 unexplained content/provenance drift fail closed。
   已有 ingestion proof 后，canonical no-op 仅在 planner、result ID、其自身
   epoch 的 identity、empty transition sets 与 source/reason 全部验证时证明后续
   trust/ack transition；input topic 是 request provenance，可合法不同于 result
   topic。它推进 knowledge time，但不能独立证明初始 ingestion。
   `govern_memories`、Web archive/restore、scope cleanup、save/Markdown、
   candidate apply、TTL expiry、soft supersede、preference removal 与 stale archive
   的每个 production status mutation 都经 canonical lifecycle service，从 durable、
   memory/time-indexed `memory_lifecycle_ledger` 重建；status 与 next version
   同 transaction，event 仅 optional audit mirror。previous 必须连续、terminal
   必须等于 current。Web row 复制 operation binding 并 exact bind durable
   `api_mutation_requests` resource/action/schema/response/status/time；audit ID
   仅 correlation。两 history ledgers indefinite retention、无 `events` FK/cascade，
   30-day cleanup 与 event ID reuse 不影响 proof。equality 使用最后一个 new
   status；unsupported/unrecorded action/status、
   gap/fork/contradiction/ledger mismatch 返回
   `unreconstructable_memory_lifecycle`。v2 启动不得留下任何 uninstrumented
   status writer；否则 release no-go。
   所有 nonce/fingerprint/digest 列还必须以 `typeof(...)=text` 拒绝可通过长度/GLOB
   外观检查的 BLOB；两 ledger fingerprint 是 NOT NULL 且严格 lowercase 64-hex，per-memory/
   source unique 无 NULL bypass。每个 writer 在 mutation 前 append stable、
   nonblank request/operation intent；ledger hash 绑定 strict request hash、result
   ordinal、predecessor 与 exact typed OLD/NEW，不绑定 trigger 当时尚不存在的
   final response。全部 output 完成后 append cross-memory result bindings 与
   strict result-hash commit seal；deferred constraint 禁止 unsealed intent commit。
   六类 INSERT 不使用 post-insert memory ID 作 retry identity。save request hash
   覆盖 `SaveMemoryRequest` 全部 raw 字段（含 local-copy/claim/ack flags/source/
   path/pattern、Option presence、files order/duplicate、exact title/content bytes）
   与 writer 的 derived values；result hash 覆盖 exact final response 和所有
   memory/operation/ledger/claim/ack/local-copy/next-step outcomes。相同 identity
   的新 content 不是 retry；Markdown 仅对 parser 真正 canonicalize 的字段规范化，
   stable source/no-source semantic identity 在 importer metadata 回写前后不变。
   pre-commit crash 无 committed DB state，local-copy staging 有显式 recovery；
   response-loss/concurrent exact retry 复用 committed
   winner，不新增 memory/version/event/operation/knowledge；same ID different
   payload 冲突，same-second distinct transition 仍按 predecessor/version 排序。
   user-context `edit` 是例外的版本化路径：
   writer 保留旧 row、在 transition epoch 将其标为 `superseded`，并插入带
   `supersedes_claim_id` 的新 row；`as_of` 早于 transition 时必须从这条完整
   version chain 恢复旧 claim，等于或晚于 transition 时才使用新版本。ClaimView
   使用各 version 的 state knowledge；继承 SourceRefs 永远使用首次引入 exact
   kind/refs 的 provenance-root binding。后写入的 transition
   `updated_at_epoch` 只能作 boundary，不能重新绑定/洗白 evidence。candidate
   replacement/no-op 可以在同一 epoch
   Supersede 多个 same-identity active rows，而只给一个 row 写显式 successor
   link；projection 必须用 authoritative candidate/result/timestamp pattern
   恢复全部 co-predecessors，不能把合法 writer state 当 fork，也不能接受无法
   解释的 unlinked Superseded row。
   其他没有 canonical governance/version history 的原地 `suppress`、
   `unsuppress` 与 `delete` mutation；若其
   `updated_at_epoch > as_of` 且没有可验证 successor，Phase A 必须保守排除或
   返回 `Unknown`，不得把 post-cutoff status/content 应用到历史查询。
   captured evidence 必须同时满足 source time
   `COALESCE(reference_time_epoch, created_at_epoch) <= as_of` 与 remem knowledge
   time（v2-frozen `inserted_at_epoch`）`<= as_of`；source event 虽早、但在
   `as_of` 后才被 ingest 的 late evidence 不得回溯改变历史 winner。若底层数据没有足够历史
   信息恢复过去状态，必须暴露限制或返回 `Unknown`，不得根据当前值伪造历史
   truth。v2 output 必须同时序列化 `requested_as_of_epoch` 与实际使用的
   `reference_epoch` 及 replayability。显式 `as_of` 是 `Exact`；
   `as_of=None` 只采样一次 now，无 proof current binding 参与输出时必须标记
   `CurrentSnapshotOnly`，该 epoch 仅可审计、不能作为 replay key。
   同一 `(host_id, session_id, event_id)` captured-event identity 的 idempotent replay 不得覆盖首次
   creation、insertion/knowledge 或 reference/source epoch；只能追加独立 keyed
   Git evidence/extraction work。既有 pre-v2 row 以当前 stored insertion 作为
   保守 knowledge floor，不得猜测或回填更早 eligibility。
5. CT-005 在同一 subject 的候选 claim 中，时间上生效的显式
   `Supersedes` 必须优先于纯 recency，并在结果中记录被替代项和选择原因。
6. CT-006 verified evidence 必须优先于 model-generated 或 untrusted
   evidence；任何 stored confidence 都不得替代这条可解释的 trust 规则。
   captured-event trust 必须复用 canonical `SourceTrustClass` 的事件语义，不能
   仅因 `tool_name` 非空就标为 Verified。WebFetch/WebSearch、任意 `mcp__*`
   与抓取网络内容的 Bash event 都是 `external_content`。分类必须验证并读取
   exact `<=16384` bytes 的 canonical `raw_keep` inline full content，或校验
   `>16384` plain UTF-8 `raw_compact` blob 的 byte counts、preview、event SHA-256
   及 current/legacy blob hash 后重建 full content；不能只读 stored preview。
   非法 storage/encoding/length/preview/hash fail closed。truth 只暴露/复用
   capture pure helpers 与 canonical pure classifier；除 duplicate timestamp
   immutability guard 外不改变 writer。memory 的有效 trust
   是“可用 evidence 中的最强 tier”与 effective source cap 的较弱者；effective
   cap 同时取 validated stored `memories.source_trust_class` 和所有 referenced
   event 重新按 canonical classifier 分类后的最弱值，防止 v060 legacy/default
   `local_tool_output` 掩盖实际 external refs。`external_content` 与 `pack`
   cap 为 Untrusted，`local_tool_output`、`repo_file`、`user_prompt` cap 为
   Verified。SourceTrustClass diagnostic evidence 不参加 strongest-evidence
   max，且 cap 本身不得把没有 verified evidence 的 claim 抬级。未知
   source-trust class 必须 contextual error。这样 mixed evidence 中的低信任
   外部来源不能被另一个 tool event 的 max 聚合提权。candidate-backed memory
   还必须把 candidate stored source cap 纳入 min，后续 memory trust rewrite
   不能提权。
   memory 引用的 captured event 必须通过 canonical project identity 证明属于
   该 memory 的 provenance source：优先使用非空 `memory.source_project`，
   只有未声明 source/routing 的 legacy row 才可按明确 fallback 使用
   `memory.project`；已路由但 source 缺失或不一致必须 fail closed。captured
   event project 与该 expected source project exact match；foreign-project
   evidence 不得被降级后继续，也不得抬高本项目 claim。memory/Observation
   `evidence_event_ids` 的每个 event source/knowledge time 还必须不晚于 enclosing
   memory version/Observation creation binding epoch。timestamp 为秒级，equality
   eligible；Phase A 无法区分同秒内的后写，若需绝对顺序，Phase C 必须持久化
   attachment sequence。
   非空 memory evidence 的 linked candidate 还必须有 completed status、exact
   origin trust cap、confidence/persisted copied fields 与 result-operation
   completion；candidate input scope 按 validated route 映射为 memory scope
   （user=>global，其余=>project），不要求 scope copied-exact。derived title
   未存于 candidate row，也不参加 equality。pending/rejected/quarantined 或
   损坏 link 不能洗白 provenance。
   user-claim source kind/ref grammar 必须 total。canonical
   `source_kind=user_context_candidate` 只能引用一个 authoritative candidate
   wrapper；inline nested source kind/refs 与 candidate row、result row copied
   fields、后续 edit preserved fields 必须 exact 且绑定到首次引入 refs 的
   provenance root，不能在 edit 时重绑。每个 candidate 的 `result_claim_id`
   指自己的 initial result；top-level result 才必须是 current claim/ancestor，
   nested wrapper 校验自己的 root/edit chain。candidate 必须有
   nonblank host/project/session，event/summary exact 同三元；terminal refs 与
   single-wrapper recursion 是 closed grammar。explicit-user event 必须 first-party。
   现 schema 无 summary→完整 generated-surfaces immutable binding，因此每个
   structured summary ref（任何 status/ack）都在 content/trust 使用前返回
   `unverifiable_session_summary_provenance`。extra fields、conflicting duplicate、
   cycle、missing/foreign/future-knowledge ref 都 fail closed。
7. CT-007 当有效的 `Refutes` 或等价冲突无法安全裁决时，结果必须是
   `Contradicted`，并保留冲突双方及相关 relation；不得静默折叠成单一 truth。
   canonical writer 通过 operation-backed `memory_edges.conflicts` 记录的
   cross-topic preference conflict 属于有效冲突，但仅当两端都是同 owner、
   同 normalized scope、同 normalized branch 的 `memory_type=preference`
   claim，且两端在各自 subject slot 内仍 surviving；任一条件不满足都不得扩张
   decision domain。只有 candidate/operation 两个 source ID 都为 NULL 才是
   unbacked；其 conflict 可以 decision-neutral。candidate-only 是 integrity
   error，operation-only 合法但必须验证 operation。edge 声称由
   `memory_operation_log` 支持后，
   missing/wrong operation、malformed `conflicting_ids` JSON、非整数 element
   或 result/conflicting endpoint membership 不成立必须返回含
   operation/edge/endpoints/field 的 contextual error，不得静默抹去 durable
   conflict。合法的 canonical pairwise graph/dream conflict 可以连接不同
   owner 或 memory type；writer 此时使用 `repo`/source-project/`memory`
   fallback operation metadata。该结构在 endpoint membership 与 canonical
   operation provenance 有效时不得报错，但必须保持 decision-neutral。只有
   两端实际满足同 owner/scope/branch 的 preference exception 时，才要求
   operation metadata 与该 uniform identity 一致并进入 cross-topic 冲突裁决。
   uniform conflict graph 必须是 matching；A-B 与 A-C 同时存在时 contextual
   error，不得任意选 pair。
8. CT-008 无匹配 claim、所有 claim 均无 current standing、证据不足或未知
   状态无法安全解释时，必须返回 abstention/`Unknown` 或明确空结果，不得
   发明 claim，也不得把失败伪装为正常空数据。
9. CT-009 每个非空 truth 或 conflict 结果必须携带 canonical ref、可用的
   evidence refs、相关 supporting/contradicting relations，以及机器可判定的
   `TruthSelectionReason`。同 scope 的 cross-subject provenance-only relation
   在连接 winner 时必须保留；“不同 subject 不参与 winner 决策”不得被错误实现
   成“丢弃所有 cross-subject provenance”。`observations` 必须经 adapter
   映射为 versioned、canonical-ref-bearing evidence view，
   保留 lifecycle、project/branch、source/knowledge time 与 captured-event refs；
   NULL refs 表示 empty，非 NULL refs 严格解析；NULL creation epoch 是 contextual
   error，不能从 display text 猜 epoch。nonlegacy row 还要求
   host/project/session 三元完整且 event exact 同三元；partial/cross-session
   fail closed。empty refs 的 trust 明确为 ModelGenerated。active historical row 必须经
   current generated-surface scanner clean 后才是 Validated。Observation trust
   最高为 ModelGenerated，supporting source 有 external 时降为 Untrusted。
   未附着 observation 进入稳定排序、去重后的 `evidence_catalog`。observation
   本身不获得 canonical Claim standing；只有同 scope、在 reference time
   生效且同时带 `source_memory_id`/`source_observation_id` 的 bitemporal
   `memory_facts` row 才是 observation evidence 附到 memory claim 的显式
   link。该 fact 的 caller-supplied `learned_at_epoch` 与实际插入时写入的
   NOT-NULL `created_at_epoch` 都必须 `<=as_of`；NULL/missing legacy 字段没有
   fallback，late insert 不能用 backdated learned time 回溯附着。不能根据共享
   event、文本相似度或模型猜测自动关联。subject selector
   只过滤 truth subjects，不过滤 scoped `evidence_catalog`。
   current snapshot 排除 stale/compressed row；explicit history 遇到 cutoff
   前已存在、但现在 stale/compressed 且没有完整 validated transition history 的
   scoped row 时必须返回 `unreconstructable_observation_lifecycle`，不能静默丢
   evidence 或改变 winner。
10. CT-010 generated enrichment 不得仅凭模型生成身份创建或覆盖 canonical
    Claim。Phase A 的读取结果必须排除不具备 claim standing 的 enrichment；
    writer 侧的统一防线属于 Phase C，Phase A hardening 不宣称已完成。
11. CT-011 stale、expired、deleted、candidate、suppressed 和 archived 状态
    必须分别处理并有独立可观察结果；未知状态必须 fail closed，不能 panic、
    自动视为 current 或静默降级。Phase A 的实际 claim sources（memory 与
    user-context claim）遇到 unknown raw status 必须返回包含 table、canonical
    ref 与 raw status 的 contextual error，不能只映射为不可见 `Unknown` 而
    没有 diagnostic。`observations.status=poisoning_quarantined` 必须明确映射为
    `(Candidate, Unknown, Live, Suppressed)`，不得进入可用
    `evidence_catalog`、claim attachment、trust aggregation 或 current truth；
    其他 unknown observation status 同样 fail closed。除 row status 外，
    adapter 必须应用
    `memory_suppressions` 的 memory-ID、topic、entity、pattern 与 user-claim
    policy targets，并映射为 `Visibility::Suppressed` 而非删除或判 false。
    canonical `user_candidate` 与 `summary` target 也必须识别和验证；Phase A
    因其没有 Claim standing 而 non-applicable，不得 transitively 隐藏 promoted
    claim 或 SourceRef，也不能把合法 row 当 unknown kind。
    当前查询只应用 `status=active` 的 suppression；historical `as_of=t` 应用
    `created_at_epoch <= t` 且在 t 尚未 revoked 的区间
    （active row，或 revoked row 的 `t < updated_at_epoch`）。边界与 revocation
    后恢复可见性必须有测试，不得只检查 canonical row 的 raw status。
    suppression owner pair `(NULL,NULL)` 表示 global，两个字段同时非空时只对
    exact canonical owner 生效；partial pair 必须 contextual error。direct
    memory/user-claim target 还必须验证 target row 属于该 owner，topic/entity/
    pattern target 也不得跨 owner 隐藏 truth。
    `memory_entities` 没有 link history；任何 applicable current entity target
    都使 projection 为 `CurrentSnapshotOnly`。explicit historical query 中有效的
    entity target 必须返回 `unreconstructable_entity_link_history`，除非 durable
    history 能证明每个 scoped memory 的 membership/non-membership。
12. CT-012 archived evidence 可用于后续 historical explanation，但默认不得
    进入 current context。Phase A 可以保守排除 archived current truth；
    historical explanation 的完整消费契约属于后续阶段。
13. CT-013 Phase B 的 Context Bundle 只能消费 versioned projection 输出，
    并分别呈现 current truth、decision 和 conflict；切换期间必须保留可回滚
    的旧 context path，不能把 projection 失败降级成看似成功的缺失 context。
14. CT-014 Phase C 只有在 read model、冲突/abstention 行为和 benchmark
    稳定后才能评估 Phase A narrow route/lifecycle substrate 以外的一般 writer
    收敛；收敛不得破坏 canonical ref、provenance、scope 或历史解释。
15. CT-015 projection 必须是只读、可重建且可版本化的。读取失败、schema
    不兼容或 evidence 解析失败必须返回可诊断错误或明确的 fail-closed 结果，
    不得修改 canonical 数据来完成查询。所有 SQL stage 必须处于一个 SQLite
    read snapshot：autocommit 入口拥有 deferred BEGIN 与 terminal
    COMMIT/ROLLBACK，caller transaction 则被复用且不得由 projection commit。

## 验收标准

### 已合并 baseline：PR #939

PR #939 是已合并的 Phase A baseline evidence：提供 versioned DTO、lifecycle
mapping、read adapter、deterministic resolution 和 18 个 truth tests。它不是
本轮 fresh verification，也不覆盖以下 hardening 验收项。

### 下一份 Phase A hardening follow-up

- [ ] Public integration test 通过真实 crate path `remem::truth` 构造 Project/
      Owner query；README migration 不出现 package-name 派生的错误 Rust path。
- [ ] relation 两端都必须属于本次 query 的 scoped claim set。
      `Supersedes` 与普通 `Refutes` 只有同一完整 subject identity 才能改变
      resolution；cross-project、cross-owner、cross-scope、
      explicit-branch-scope 外与未授权 cross-subject decision relation 不影响
      winner。唯一例外是 operation-backed canonical `memory_edges.conflicts`
      对同 owner/scope/normalized-branch preference survivors 的 cross-topic
      `Refutes` post-pass：两个 subject outputs 都必须为 `Contradicted` 并保留
      双方与 relation，但各自 identity 不合并。连接 winner 的 scoped cross-subject
      `Supports`/`DerivedFrom` 作为 provenance 输出保留但不参与选择，并有正反
      fixtures。`branch=None` 的 branch-agnostic 全分支语义和
      `branch=Some(B)` 的 neutral-plus-exact 语义都有独立 regression。
- [ ] `memory_edges` 与 trusted memory-to-memory `graph_edges` lookup 在 SQL
      或等价 bounded lookup 中受 scoped IDs 约束，不扫描无关项目全表；提供
      seed-933 chunk-boundary/high-fanout/unrelated-project structural assertions
      与 final-head JSON record；p50/p95 只记录，不冒充跨机器 hard threshold。
      scoped raw `memory_edges` 必须在 endpoint 输出过滤前按上述六种 kind total
      mapping；六种方向各有 fixture，unknown kind 返回 table/edge/raw error。
- [ ] `ClaimRelationKind::Supports` 有真实 adapter/output fixture。
- [ ] `observations` 经 read adapter 映射为不具 Claim standing 的 versioned
      evidence；project/branch/lifecycle、source/knowledge time、
      captured-event refs、explicit-link-only attachment 与 malformed provenance
      均有 focused fixtures。v2 DTO/golden 必须锁定
      `EvidenceKind::Observation`、`CurrentTruthProjection.evidence_catalog`、
      canonical `observation:<id>`、stable ordering/dedup、subject-filter
      independence，以及 `memory_facts(source_memory_id,
      source_observation_id)` 的唯一 attachment direction。NULL refs、active-row
      read scan、external supporting trust，以及 fact learned/created/valid/
      invalidated/replacement before/equal/after、late-insert/backdated-learned
      rejection 都有 regressions。cutoff 后
      stale/compressed 且无完整 transition history 必须显式 integrity error。
- [ ] `poisoning_quarantined` observation 映射为
      `(Candidate, Unknown, Live, Suppressed)`，并有 lifecycle 与 catalog/claim
      attachment negative regressions；其内容不得进入 usable evidence 或 trust
      resolution。其他 unknown observation status 返回 contextual error。
- [ ] Observation nullable `created_at_epoch`、完整/legacy
      host/project/session identity、四种 poisoning status 均有 fixture；
      structured session-summary 的 missing/safe/quarantined/acknowledged 全部返回
      `unverifiable_session_summary_provenance` 且 `total_changes=0`。
- [ ] 每个 memory evidence ref 通过 `captured_events.project_id` join canonical
      `projects.project_path`，必须与非空 `memory.source_project` exact match；
      仅无 routing assertion 的 legacy row 可 fallback 到 `memory.project`，
      routed/partial ownership row 缺 source、无法解析 project identity 或
      foreign-project ref 都返回含 memory/event/expected/actual project context
      的错误，不能静默降级 trust；memory candidate completion、Observation/
      user candidate exact host/project/session 和 epoch-boundary fixture 全覆盖。
- [ ] explicit `as_of` 查询只使用 source time 与 knowledge time 都不晚于
      `as_of` 的 memory claim row、user-context claim row 和 captured evidence；
      memory operation proof allowlist、exact fields、earliest
      `(created_at_epoch,id)` 与 current-snapshot-only fallback 决定 effective
      knowledge；candidate-backed memory 仅对 initial completion identity 做
      candidate exact-match，cutoff route state 只决定 membership/emitted
      `SubjectIdentity`；另验证 origin trust cap 与 unexplained mutation error；operation-less
      procedure memory 有 current/historical 正反 fixture。UserContextClaim 则区分 immutable version creation、
      predecessor transition 与可重建的 current state。ClaimView/SourceRef
      temporal fields、old-source/new-ingest memory、
      source-before/knowledge-after evidence、user claim post-cutoff
      edit/suppress/unsuppress/delete、各时间的 before/equal/after boundary 与
      historical winner都有 regression。candidate replacement/no-op 的
      multi-active co-predecessor before/equal/after、kept-result SourceRefs 与
      unexplained unlinked-row error 也必须覆盖；row 后来原地更新且无法重建旧
      版本时必须排除/`Unknown`。same/cross-topic canonical noop、candidate ack、
      general/Web governance、scope archive、cleanup-plan active/stale 的统一
      lifecycle order 与 before/equal/after 均覆盖；unsupported/unrecorded、
      gap/fork/Web ledger mismatch 返回指定 lifecycle error。读取使用
      lifecycle memory/time index；30-day event cleanup 后两 ledger、Web proof
      与 serialized history 完全不变。每类 writer 还覆盖 strict fingerprint/
      nonblank request-ID DDL、pre-write intent→trigger-v1→final seal 时序、
      cross-memory result mapping、exact retry reuse、different-payload conflict、
      same-second order、unsealed commit rejection、crash-before-commit 与 commit-
      success/response-loss/concurrent retry，且不重复 memory/mirror/knowledge；
      save 的 raw CRLF/whitespace、local-copy/claim/ack 与 file order 均不碰撞，
      Markdown 只跨 importer-owned metadata rewrite 保持稳定。
      duplicate captured-event replay 前后 timestamp/历史输出不变与
      replayability serde golden 均覆盖。
- [ ] captured-event trust 使用 canonical source classification；WebFetch、
      WebSearch、`mcp__*`、external-fetching Bash、`pack`、unknown class 与
      stale legacy/default cap mixed-evidence regressions 证明 external/pack cap
      不能被 tool event 提权，SourceTrustClass 不自抬级，unknown class fail
      closed，cap 也不能无证据抬级；>16 KiB network-Bash、blob encoding/hash
      failures，以及 raw_keep/current-hash blob/legacy-hash blob positives 均有
      regression；覆盖 16384/16385 bytes、multibyte boundary，且网络标记仅位于
      preview 遗漏的 middle。workspace candidate scope mapping 与 pack title
      exclusion 也有 positive regression。
- [ ] operation-backed cross-topic conflict provenance 必须解析并验证
      `memory_operation_log`；malformed `conflicting_ids` JSON、wrong element
      types、missing/wrong operation、replacement/pairwise endpoint membership
      或 owner/type inconsistency 均返回 contextual error。只有两个 source ID
      都为 NULL 的 edge 可保持 unbacked；candidate-only error、operation-only
      valid 各有 fixture。canonical
      graph/dream pairwise heterogeneous operation 在结构与 endpoint membership
      有效时也保持 decision-neutral；其 fallback owner/type metadata 不得被误判
      为损坏 provenance。uniform preference exception 另行严格校验
      owner/scope/branch/type，并覆盖 A-B + A-C overlap error。
- [ ] malformed `evidence_event_ids`、malformed `source_refs_json`，以及
      syntactically valid 但 dangling 或 foreign-project 的 captured-event ref
      均 fail closed，并返回包含 claim/ref/project context 的可诊断错误；不得
      静默形成较低 trust 的“正常成功”。
- [ ] closed user source-kind mapping 覆盖 canonical 顶层
      `user_context_candidate`；每种 structured ref 的 exact fields/type、
      extra-field、scope/binding/reference-time 和 duplicate rules 都有 fixture。
      candidate wrapper 的 authoritative candidate/result copied fields、exact
      owner/host/project/session、preserved edit edges、initial-result ancestry、
      nested own-result chain、provenance-root binding、manual scalar path `0`、
      single-wrapper recursion/cycle 与 first-party predicate 均有正反测试。
- [ ] SQLite authorizer、`total_changes` 允许 owned BEGIN/COMMIT/ROLLBACK 但拒绝
      DML/DDL；WAL concurrent-writer regression 证明 projection 全程单一 snapshot。
- [ ] memory subject identity 与 canonical writer 对齐为
      `(source, owner_scope, owner_key, normalized memory scope, memory_type,
      NULL/exact-empty=>memory:<id> singleton，否则 exact topic_key)`；相同
      topic、不同 owner/scope/`memory_type` 绝不
      竞争或 supersede。owner pair 必须来自完整 canonical fields；仅两者同时
      缺失的 legacy row 可按 v019/default writer rules 一起 fallback，partial
      pair fail closed。Project owner-key/target-only、repo reroute old→new、
      stale non-repo placement exclusion、source-only exclusion、Owner union、
      canonical/legacy/repo-rerouted global、legacy non-global、Owner/Project branch semantics
      和 user-claim-only compatibility wrapper 都有正反 regression；wrapper
      只读 applicable user-claim/pattern suppressions，malformed exact-owner
      memory/memory-only suppression 不改变其 result/error domain。Project 与
      indexed route ledger 的 migration/backfill、coverage probe、atomic writer
      guard 与 A→B→C intermediate-B discovery 均有 regression；Project/Owner 的
      route before/equal/after 使用历史 owner/target/scope/type/raw nullable key。
      normal save 只对真实可达的 same-type raw-key update 做 before/equal/after；
      stable-source-ID Markdown project→global 同时改变 type/key，覆盖 cutoff 前
      旧 Project identity、equality/之后新 Owner identity、`markdown_import`、
      atomic rollback 与 missing-predecessor/legacy
      gap error；incomplete/forward-only pre-floor chain 不能使用 current。
      六类 INSERT、normal save/Markdown/scope-cleanup 三类 UPDATE、同值 no-op、
      changed-route staging 与 bypass rejection 均有 focused regression。
      `topic_key` 为 NULL 或 exact-empty 时必须使用 `memory:<id>` singleton；
      `foo`、` foo ` 与纯空白 key 保持不同 slot。该合法输入变化触发上一轮既定
      version boundary：hardening 输出必须为 `projection_version=2`，
      DTO/module paths、v1-to-v2 intentional golden diff 与已发布 v1 的迁移说明
      都在 exact packet 中批准；不得继续标 v1 或无审核改写 golden。v2 必须在
      `0.7.0`（或实现时下一个明确的 breaking SemVer boundary）发布，不能作为
      `0.6.x` patch 偷渡。
- [ ] unknown memory/user-claim raw status 返回包含 table/canonical-ref/raw-value
      的 contextual error，并覆盖每个实际 claim-source adapter；已知 status
      的 total mapping 不回归。
- [ ] `memory_suppressions` policy table 的 memory-ID、topic、entity、pattern
      与 user-claim targets 均映射为 `Visibility::Suppressed`；合法的
      user-candidate/summary target 被验证后保持 Phase-A non-applicable。
      current 与 historical created/active/revoked interval（含等值边界）
      regressions 证明 suppression 不被当作 false，也不会在 revoke 后继续隐藏；
      `(NULL,NULL)` global、完整 owner pair exact-match、partial pair error 与
      direct/value target 的 cross-owner negative fixtures 防止跨 owner 隐藏。
      entity current-only replayability、post-cutoff add/replace link 与 explicit
      historical `unreconstructable_entity_link_history` 均有 regression。
- [ ] final rebase、current-contract/version metadata 更新全部完成后，在最终
      candidate head 重跑完整 bounded-query/performance/golden validation；evidence
      绑定 exact SHA 与 relevant migration/index/truth/dependency fingerprints，
      其后任何 commit/rebase 都使 evidence 失效并要求重跑。
- [ ] follow-up PR 使用 `Refs #933`，只修改批准的 Phase A hardening、当前
      `docs/specs/GH933/{PRODUCT,TECH}.md` 契约与必要的 release metadata；
      current contract 必须移除已被 hardening 纠正的过时完成声明，不接入
      Context Bundle 或一般 writer convergence；schema/writer 范围仅限 reviewed
      route/lifecycle ledgers migration/backfill、atomic instrumentation 与
      duplicate capture timestamp immutability，不关闭 GH-933。

### GH-933：后续整体完成条件

- [ ] Phase B 让 Context Bundle 消费 projection，并验证 current truth、
      decisions、conflicts 与旧 path rollback。
- [ ] worktree/task selector 具备与 project/branch 一致的 scope isolation
      和不泄露验证。
- [ ] archived evidence 可参与 historical explanation，但默认不进入
      current context。
- [ ] Phase C 对 Phase A narrow history substrate 以外的一般 Claim writer
      收敛决定有 benchmark、迁移/兼容与 rollback 证据；若不收敛，也必须记录
      明确决定和边界。
- [ ] Phase C 在启用 session-summary ref 前持久化完整 generated-surface snapshot
      或等价 immutable binding，并在需要绝对顺序时持久化 attachment sequence。
- [ ] generated enrichment 在最终写入与读取契约中都不能创建或覆盖
      canonical Claim。
- [ ] 文档、调用方契约与发布说明同步后，才可声明 GH-933 整体完成。

## 边界情况

- 两个 claim 时间相同、evidence trust 相同，且没有可裁决的关系。
- 新 decision 在 `as_of` 之后 supersede 旧 decision。
- user-authored evidence、tool output 与 model-generated enrichment 冲突。
- branch A 与 branch B 对同一 subject 有不同 current truth；显式 branch 查询
  只消费 neutral+exact rows，而 `branch=None` 明确形成 branch-agnostic 竞争。
- claim 在 `as_of` 前存在，但其 verified captured-event evidence 在 `as_of`
  后才被 remem ingest，source time 可能仍早于 `as_of`；或引用的 ledger event
  已不存在/属于另一个 project。
- 同一 captured-event 在 cutoff 后 idempotent replay；原 insertion/reference
  timestamps 与 cutoff 前 truth 必须保持不变。
- imported memory 的 source-created/reference time 早于 `as_of`，但最早
  route-at-operation-compatible canonical operation proof 晚于 `as_of`；或只有历史
  incompatible operation/既有 row 在 `as_of` 后原地更新。
- procedure memory 带 nonempty event refs 但没有 operation log：current query
  以 reference epoch 绑定，explicit historical 必须排除/`Unknown`。
- user-context claim 在 `as_of` 前创建后被 edit；完整 supersedes version chain
  必须恢复 cutoff 前旧 claim。suppress/unsuppress/delete 的原地 mutation
  没有等价历史 row 时才保守排除/`Unknown`。
- 两个不同 `memory_type` 使用相同 `topic_key`；它们必须形成不同 typed
  subjects。
- 两个 memory 分别使用 NULL 或 exact-empty `topic_key`；它们必须各自形成
  `memory:<id>` singleton。
- `foo`、` foo ` 与纯空白 `topic_key` 都是不同的 byte-exact topic slots。
- global、workspace 与 project memory 使用相同 type/topic，但 owner/scope
  不同；它们必须形成不同 subject identities。
- routed memory 的 `memory.project` 是 target/synthetic project，而 captured
  event 属于非空 canonical `source_project`；它是合法 provenance。缺失 source
  的 routed/partial row 则 fail closed，不能 fallback 到 target。
- canonical writer 记录两个不同 topic keys 的同-owner/scope preference
  conflict；两个 endpoint 在各自 slot surviving 时两个 outputs 都必须
  `Contradicted`，但一个任意 cross-topic conflict edge不得扩大 decision domain。
- A-B 与 A-C 都是 uniform preference survivor conflict；查询必须报 ambiguous
  matching error，不能让遍历顺序决定 B 或 C。
- conflict edge 带 `source_operation_id`，但 operation 的 `conflicting_ids`
  不是合法 integer-ID array，或 result/conflicting endpoint membership 与 edge
  不一致；查询必须 contextual error，不能当作普通 unbacked edge 忽略。
- relation 仅带 candidate ID 而没有 operation ID 时必须报错；两者都没有才是
  unbacked，只有 operation ID 时走完整 operation validation。
- canonical pairwise conflict 的 endpoints owner/type 异构，operation 使用
  writer-defined fallback metadata；结构合法时不得让整个 projection 失败，
  也不得扩大 preference decision domain。
- scoped `Supports`/`DerivedFrom` 连接 winner 与另一个 typed subject；该
  provenance 保留，但不能改变 winner。
- memory 或 user-context claim 存有未识别 raw status。
- active/revoked `memory_suppressions` 通过 memory/topic/entity/pattern/
  user-claim target 覆盖 canonical row status，并在 `as_of` 区间边界改变
  visibility。
- Observation 在 cutoff 后原地 stale/compressed，或 entity link 在 cutoff 后
  add/replace；缺少完整 transition/link history 时历史查询必须显式失败。
- memory 在 cutoff 前后经过 general governance 或 Web archive/restore；完整
  audit/ledger chain 恢复 old/new status，gap/fork/contradiction 必须显式失败。
- 任一 writer 在 transaction commit 前 crash，或 commit 已成功但 response 丢失/
  concurrent duplicate；前者零残留，后者按 pre-write ID 复用跨 memory exact
  result，不推进 knowledge；same ID/different content 必须 conflict。
- fact 在 cutoff 后实际插入但 caller 把 learned time 回填到 cutoff 前；不得附着。
- scoped `memory_edges` 存在 typo/newer edge type，或 known `derived_from` 只有
  provenance endpoint；前者 contextual error，后者不得伪造 Claim endpoint。
- stable-source-ID Markdown existing-row import 把 memory 从 project 改为 global
  并改变 type/raw nullable key，normal save 只改同 type raw key，或 scope cleanup 改路由；
  before/equal/after 必须使用完整旧/新 route/identity，
  缺 predecessor/legacy coverage 不得 fallback current row。
- writer 在 claims 与 relation/evidence/suppression 分段读取之间提交；进行中的
  projection 只能看到 transaction 开始后的一个 SQLite snapshot。
- claim 被 suppressed 但仍可能在政策允许的历史审计中存在。
- archived evidence 仍有历史价值，但当前查询不应把它当作 active truth。
- 底层状态被原地改写或数据已删除，无法完整重建过去时点。
- 空 scope、无匹配数据、未知 stored status、损坏 evidence ref 或读取失败。

## 发布说明

PR #939 的 Phase A library-level v1 已随 GitHub Release 与 crates.io
`remem-ai` 0.6.26 公开发布，0.6.27 仍包含该 API；它没有新增 CLI/HTTP 或
Context Bundle 用户界面。下一份 Phase A hardening follow-up 仍以 `Refs #933`
关联开放 issue。由于 ownership-aware identity、Observation catalog 与 DTO shape
会 source-break 直接使用 `remem::truth` v1 的调用方，v2 必须在 `0.7.0`
（或实现时下一个明确的 breaking SemVer boundary）发布，并在 README、当前
架构文档、changelog 与全部 distribution metadata 中提供 v1→v2 迁移说明。
Phase B/C 必须独立经过架构审阅、实现验证和发布说明；在这些阶段完成前，不得
将 GH-933 或完整 CurrentTruth 能力标记为已交付。
Breaking migration 不得由普通 open、CLI、hook、worker、MCP/API 或通用
`run_migrations` 自动执行；operator 必须先生成绑定 DB identity、binary、backup
destination、nonce 与 expiry 的 mode-0600 durable plan，再以 exact lowercase
SHA-256 显式 apply，且 approval durable single-use。任何 mismatch/reuse 必须在
首次 live write 前失败。
