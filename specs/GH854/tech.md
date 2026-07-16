# Tech Spec

## Linked Issue

GH-854

## Product Spec

[`product.md`](product.md)

## Spec 状态与实施门禁

本 tech spec 是 Draft，不构成 `spec_approval` 或 implementation 授权。当前 `origin/main`
缺少 issue 引用的 `docs/research/agent-memory-optimization-research-2026-07.md`，现有代码也没有
可跨 Lessons、排除 Core 后的 MemoryIndex view、Sessions 比较的共同任务相关性分数。以下设计固定边界、状态机、
证据形状和验证方式，但共同评分器、精确阈值、弱查询判定、运行时上限、实验样本规模、reference
model/runner/environment、非回归预算与默认 k 必须先由 human `spec_approval` 通过 spec amendment
冻结。任一项未冻结时不得写 `tasks.md`、进入 `ready_to_implement` 或更改运行时默认。

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| 分区与预算策略 | `src/context/policy.rs:24`, `src/context/policy.rs:40`, `src/context/policy.rs:143` | 默认总预算 12,000 chars；MemoryIndex 50/4,000、Core 6/3,000、Sessions 5、Lessons 4/1,200；`ContextPolicy` 汇总分区策略 | GH-854 必须扩展现有策略，而不是建立并行 gate |
| Core 门槛 | `src/context/sections/core.rs:13`, `src/context/sections/core.rs:58` | 额外 Core 使用固定 intrinsic score 门槛，并至少保留首项 | Core 不是本 issue 的共同任务相关性范围，避免误改 |
| 三个受控 renderer | `src/context/sections/lessons.rs:64`, `src/context/sections/index.rs:78`, `src/context/sections/sessions.rs:18` | 按输入顺序在各自 item/char 限额内渲染，没有共同任务相关性门槛 | 选择计划必须在 renderer 前生成，renderer 继续负责本地预算 |
| SessionStart 编排 | `src/context/render.rs:399`, `src/context/render.rs:433`, `src/context/render.rs:475`, `src/context/render.rs:560` | 依次渲染分区、汇总 stats，再从已渲染 IDs 构建审计项 | 插入冻结选择计划并确保审计反映最终输出的主要边界 |
| 总预算裁剪 | `src/context/render.rs:550`, `src/context/render.rs:721`, `src/context/render/truncation.rs:3` | footer 后按稳定文本边界裁剪；裁剪器不返回 item stable keys | 必须把“文本看起来完整”升级为 item-aware 生存真值，避免审计把已裁剪或半截项记为 injected |
| 统计与 footer | `src/context/render/stats.rs:10`, `src/context/style.rs:45` | 只显示分区 count/chars、输出 chars 和 `truncated` | 当前无法解释相关性空白、k 丢弃和评分失败 |
| item 审计 | `src/context/audit.rs:14`, `src/context/audit.rs:233`, `src/context/audit.rs:324` | schema 已支持 `score`、status、drop reason；当前不审计 session summary，且最终截断以 title 子串回写 | 可无迁移扩展 item 审计，但要加入 summary 稳定 ID 并消除 title 匹配歧义 |
| Session summary identity | `src/context/types.rs:22`, `src/context/query.rs:512`, `src/context/query.rs:591` | `SessionSummaryBrief` 没有 ID，查询也不选择 `ss.id` | 为 Sessions 提供稳定候选和审计身份；现有表已有 ID，无 schema migration |
| 隐式查询 | `src/context/implicit_query.rs:17`, `src/context/query.rs:184` | 从 project/branch/commit/workstreams/summaries 形成 SessionStart 查询 | 是评分输入候选，但“过弱”的闭集判定尚未定义，必须在 approval 中冻结 |
| 混合检索 | `src/context/hybrid_context.rs:27`, `src/context/hybrid_context.rs:120` | weighted RRF 用于 memory IDs，最终函数丢弃融合分数并返回 `Vec<Memory>` | 不能声称现有分数可直接跨 lesson/session 比较；也不隐式依赖其他 reranker issue |
| PromptSubmit 门槛 | `src/context/prompt_submit.rs:57`, `src/context/prompt_submit.rs:76` | PromptSubmit 有单独的候选过滤与 item 审计路径 | 其评分语义不是 SessionStart 已校准共同分数，只能经新证据批准后复用 |
| PromptSubmit 审计 finalizer | `src/context/prompt_submit.rs:92`, `src/context/prompt_submit.rs:103`, `src/context/audit.rs:324` | PromptSubmit 与 SessionStart 最终都调用同一 audit writer/finalizer，但 PromptSubmit 自己构造输出与 injected items | 改 stable-key finalizer 时必须保持 PromptSubmit 空/有输出两条路径，不能只修 SessionStart |
| 重复注入 gate 身份 | `src/context/injection_gate/data_version_hint.rs:17` | data-version 包含 render contract、policy 和数据指纹，驱动已有 pre-render/duplicate gate | scorer/version/threshold/k 必须进入身份，避免策略改变仍被旧 decision 抑制 |
| strict pre-render suppression | `src/context/render.rs:147`, `src/context/render.rs:152`, `src/context/injection_gate/pre_render.rs:75`, `src/context/injection_gate/pre_render.rs:113`, `src/context/injection_gate/store.rs:133` | data-version 命中时先更新 `context_injections.updated_at_epoch`/suppress count 并直接返回空输出；render 收到空 `audit_items`，因而没有对应 item run | B-023 不能把这类正常 reuse 一律报 `audit_incomplete`，也不能用旧 injected rows 冒充本次空输出；suppression output 与 reuse item evidence 必须同事务 |
| deterministic injection eval | `src/context/render/eval.rs:35`, `src/context/render/eval.rs:105` | 使用默认 policy 生成分区 snapshot，只覆盖现有计数/截断 | 可补选择规则与空/error 的确定性回归，但不能替代 coding outcome |
| coding bench 输入与归因 | `src/eval/coding_bench/condition.rs:98`, `src/eval/coding_bench/condition.rs:214`, `src/eval/coding_bench/types.rs:246` | 已从 audit 读取 injected memory IDs，并有 injected/used/irrelevant/missing attribution | 是 k sweep 的主效果层；需增加 arm 与 SessionStart 选择证据 |
| coding bench 执行顺序 | `src/eval/coding_bench/run_plan.rs:10`, `src/eval/coding_bench/run_plan.rs:35` | 完整 condition/task/run matrix 使用运行时随机 seed shuffle，当前报告不保存 seed | 预注册 sweep 必须由 charter 固定 seed 并在报告保存 exact plan，才能复验执行顺序 |
| coding bench CLI/报告 | `src/cli/eval_types.rs:204`, `src/cli/actions/eval.rs:393`, `src/eval/coding_bench/artifact.rs:21` | `eval-coding-bench` 有 options、runner 和可验证 JSON report | 扩展受控 matrix、预注册元数据与报告校验，不另造平行 benchmark runner |
| CLI status | `src/cli/actions/query/status.rs:51`, `src/cli/actions/query/status.rs:636`, `src/cli/actions/query/status/types.rs:218`, `src/db/query/status_spend.rs:21` | `remem status`/`--json` 的 `latest_session_memory_spend` 已显示 context chars、估算 tokens、emit/suppress runs | 持久化 relevance/audit 状态应扩展此现有 surface；hook footer 只负责本次即时状态 |
| 当前审计契约 | `docs/specs/current-memory-contracts/PRODUCT.md:121`, `docs/specs/current-memory-contracts/TECH.md:97` | 要求 output/item 两层可解释，item 为 injected/dropped/abstained，空输出也需留证 | GH-854 实现必须更新当前契约而不是另立冲突规则 |
| 当前预算文档 | `docs/context-budget-design-2026-04-29.md:78`, `docs/ARCHITECTURE.md:199` | 记录 ContextLimits 与 SessionStart 数据流 | 用户/维护者文档需同步新选择顺序、状态和关闭方式 |
| 版本同步 | `Cargo.toml:3`, `plugins/remem/.codex-plugin/plugin.json:3`, `npm/remem/package.json:3` | runtime 变更触发 remem binary/plugin/npm/release manifest 同步 | implementation PR 必须通过仓库版本门禁 |

## 设计方案

### 1. 单一策略入口与关闭默认

在 `ContextPolicy` 内新增 typed `SessionStartRelevancePolicy`，而不是新建第二个注入 gate。它至少
携带 mode、scorer identity/version、共同阈值、`max_non_core_items`（k）、执行期限和 policy
schema version。公共配置键和值域在 human approval amendment 中冻结；解析必须一次性校验完整
组合，非法组合返回明确配置错误，不做字段级 fallback。

- `off` 是兼容默认：Lessons、MemoryIndex、Sessions 的输入、正文顺序和预算选择保持基线；
  footer/audit 可新增稳定的 `off` 诊断。
- `enabled` 仅在 scorer/version/threshold/k/期限全部合法且获批时成立。
- policy 是请求开始时的不可变快照。Claude Code 与 Codex 从同一 parser 和默认值获取策略。
- 不改变 Core/Preferences/Workstreams/RetrievalHints 的 `SectionPolicy`。

### 2. 共同候选与评分契约

新增请求私有的 `SessionStartRelevanceCandidate` 和 `SessionStartRelevancePlan`。plan 构造时先用
现有 `render_core_memory_with_limits_and_staleness` 的同一输入、reference time 与 limits 生成临时
Core render summary，立即冻结 `core_ids`；不得改 `sections/core.rs` 的评分、最低分、type cap 或预算。
临时 Core body 在保持现有 section 顺序的位置一次性写入最终输出，不重复选择。

共同候选闭集只来自已加载且已通过 owner/filter/staleness 等既有资格规则的 Lessons、Sessions，
以及从 `loaded.memories` 先排除冻结 `core_ids` 后形成的 MemoryIndex view：

```text
stable_key = (item_kind, item_id)
candidate = stable_key + section + source_rank + approved_projection
score evidence = scorer_id + scorer_version + finite common-scale score + query identity
decision = injected | dropped | abstained + closed reason
```

Lessons/MemoryIndex 使用 memory ID；Sessions 在现有查询中补选 `session_summaries.id` 并带入
`SessionSummaryBrief`。同一 stable key 在计入 k 前去重；Core ID、被 Core view 排除项和重复项
都不会进入 scorer 或占用 k。评分只能读取冻结的 implicit query 和候选投影，不新增网络、LLM、
模型下载或后台写入。原始 query/正文不进入普通日志或提交的报告。

共同 score 的算法、投影、取值域和跨分区校准是本 spec 的 `spec_approval` blocker。获批算法
必须证明三种分区分数可比较；`query_hybrid_context_memories` 的 RRF、PromptSubmit token overlap
或其他未合并 issue 的分数都不能在没有校准证据时充当默认实现。算法/投影/归一化任一变化都要
递增 scorer version 并产生不同 data-version。

### 3. 确定性选择与 fail-closed 状态

`enabled` 状态下采用唯一顺序：

1. 用既有 Core selector 冻结 Core IDs/body，并从 MemoryIndex view 排除这些 IDs；
2. 构造 Lessons、Sessions、排除 Core 的 MemoryIndex 候选，按 stable key 去重，同时冻结 implicit
   query、候选快照和 policy；
3. 对全部唯一受控候选完成评分并验证 finite/range/version；
4. 丢弃低于共同阈值的候选，reason=`below_relevance_threshold`；
5. 对合格项按 `score desc -> section order -> stable_key asc` 稳定排序，取前 k，余项
   reason=`k_budget`；section order 固定为 Lessons、MemoryIndex、Sessions，仅用于同分 tie-break；
6. renderer 依次应用现有 section item/char budgets，reason=`section_budget`，并保持原有可见 section
   顺序；
7. item-aware 总预算只保留完整 item，reason=`total_char_limit`；已有 duplicate/delta gate 再决定
   `gate_suppressed`/`delta_preview`，不能反写成相关性失败。

达到门槛的项不足 k 时不补齐。空/弱 query 产生三分区 `abstained`，reason=
`relevance_query_unavailable`。评分器错误、超时、NaN/Inf、版本不符或部分结果产生 status=`error`
与 reason=`relevance_scoring_failed`，并丢弃同请求全部受控评分结果；非受控分区继续按现有策略。
配置非法在进入评分前失败并记录 `relevance_config_invalid`。reason codes 是闭集，增加或改义需
spec revision。

取消/超时不会发布部分 plan。并发请求不共享 mutable scoring state；同一冻结输入重试得到同一
plan。data-version 明确包含 query identity、scorer/version、threshold、k、policy schema，防止
旧 duplicate-gate 证据覆盖新策略。

### 4. 渲染、统计与审计真值

renderer 接收不可变 plan，仅渲染 `selected` 项。受控 item 在内存 render plan 中携带 stable key 和
完整字符区间；实现可在序列化前用剩余预算选择完整 item，或让总裁剪器在稳定 item 边界裁剪并返回
最终完整存活 stable keys。两种实现都必须同时预算 header、footer 与 truncation marker，且不得把
半截 item 写入输出。

`ContextGateDecision`/delta path 必须把 total/delta/suppression 后最终完整 stable keys 交给共享
`finalize_items_for_decision`。finalizer 只按 stable key 判定 `injected`，不再搜索 title 子串；被
total/delta 裁剪的候选分别记录 `total_char_limit`/`delta_preview`，suppressed 保持
`gate_suppressed`。审计写入以单个 injection run 原子提交，禁止失败后留下看似 complete 的部分 rows。
PromptSubmit 继续复用该 finalizer，并从自己的实际 rendered memory IDs 提供 stable keys；它没有
SessionStart relevance policy，也不能因 finalizer API 变化丢失 injected/dropped/abstained rows。

strict pre-render 命中不能再只调用 `record_suppression`。新的 suppression/reuse finalizer 必须在
一个 SQLite transaction 内完成以下全部步骤：

1. 找到最近的、由新原子 writer 产生的 complete **emitted** item run；它必须精确匹配
   host/project/session/injection key、context hash、data-version、query fingerprint、policy/scorer
   version、threshold 和 k。reuse run 不得再作为源，避免无界引用链或循环。
2. 为本次 suppression 生成唯一 `injection_run_id`，复制可复验的 score/decision/stable-key
   evidence 并记录 `reused_from_run_id`。来源中的 `injected` 在本次必须转为 `dropped`，
   reason=`gate_suppressed_reuse`、render order 为空；其他 closed decisions 保留原因。因为本次输出为空，
   final injected count 必须为零。
3. 插入该 complete reuse item run，并更新同一 `context_injections` row 的 `output_mode`/
   `updated_at_epoch`/`suppress_count`；两者共用同一 event epoch 并一次 commit。

无需新列：新 writer 在现有 `provenance` 使用 closed v2 key/value envelope，至少包含
`audit_schema=2`、`run_item_count=<N>`、`data_version=<64hex>`、`query_fingerprint=<64hex>`、
`policy_id/version`、canonical threshold/k；reuse rows 另含
`reused_from_run_id_b64=<base64url-no-pad>`。未知/重复 key、非法编码或同 run 字段不一致均使 run invalid。
source complete 的定义是：同一 run 的实际 row count 精确等于每行相同的 `run_item_count`，全部 identity
字段一致，且 `output_mode` 是 materialized `full`/`delta` 而非 reuse；因此 legacy/手工半批 rows 不能成为源。

来源 run 缺失、legacy/部分、任一 identity 不匹配、copy/update 失败时整个 transaction 回滚，
pre-render 必须返回 miss 并走完整 render + item finalizer；不得留下“新 context epoch + 旧/空 item
evidence”。这是对先前 complete run 的精确、可审计复用，不是把时间差强行解释为 complete。

`ContextRenderStats` 新增 relevance state 和每受控分区的 candidate/eligible/injected 以及按闭集
reason 的 dropped counts。footer 在 `off/applied/abstained/error` 之间显式区分，并把 relevance
blank 与 truncation 分开显示。关闭模式不改 section body/order/count；footer 的新增诊断是唯一
允许的稳定可观测格式扩展。

`ContextAuditItem` 和现有 `context_injection_items` 列足够承载 score、status、drop_reason；
scorer/version/threshold/query fingerprint 写入结构化、无正文的 provenance。Sessions 使用
`item_kind=session_summary`、`item_id=ss.id`，不得伪造 memory ID。无需 schema migration。审计
写入失败由当前调用点提升为 error 级诊断；footer 的本次选择状态仍保留，但不得声明 audit
complete。

### 5. k sweep 复用 coding bench

扩展现有 `eval-coding-bench`，增加预注册的 SessionStart policy arm 维度，不新增另一套 runner：

- arms 固定为 `production_baseline,k1,k3,k5,k10`；baseline 必须使用 main 的当前关闭策略，四个
  k arm 共享一个获批 scorer/version/threshold/candidate projection。
- 现有 `remem/no_memory/curated_file` condition 语义保留；k 只作用于 remem SessionStart 注入。
- charter 是 closed-schema JSON，记录 issue、fixture/task IDs 与 hash、五个 arms、run count/order
  seed、model、runner/version、environment、scorer config、threshold、所有非回归预算，以及覆盖
  runner/scorer/runtime 可执行源码闭集的 `approved_code_commit`、sorted `approved_code_paths` 和
  `approved_code_tree_sha256`；批准的 code commit 必须是 charter target commit 的祖先，runner 要求这些
  路径从批准 code commit 到 run source HEAD 逐字不变。charter 不包含自身 commit SHA、
  tag object ID 或 `approved_by`；内容不能自我批准。
- human approval 的唯一载体是签名 annotated tag，名称闭集为
  `refs/tags/remem-approval/gh854-sessionstart-k-sweep-v<N>`，并以包含 charter blob 的 commit 为
  tag target。tag message 必须只含可解析 trailers：`SpecRail-Issue: GH-854`、
  `Approval-Kind: sessionstart-k-sweep-charter`、`Charter-Path: eval/coding-bench/sessionstart-k-sweep-charter.json`、
  `Charter-SHA256: <64hex>` 和 `Approved-By: <stable-human-id>`。轻量 tag、未签名 tag、
  额外/缺失 trailer 或从 `latest` 自动猜 tag 均拒绝。
- runner 必须显式接收 `--approval-tag <exact-name>`、`--approval-tag-object <exact-object-id>` 和由
  human/operator 提供、位于 checkout
  外的 `--approval-trust-root <absolute-path>`。trust root 使用 closed schema
  `{"version":1,"allowed_signers":[{"human_id":"...","fingerprint":"..."}]}`；runner 不创建、更改或
  从候选 branch 导入它，并记录其 hash。`git verify-tag --raw` 的 primary fingerprint 必须与
  `Approved-By` 在 trust root 的唯一映射精确相等；仅“签名有效”但 signer 不可信仍拒绝。
- 在写任何 raw row 前，runner 必须先断言 tag ref 当前解析到显式传入的 exact object ID（因此移动/
  重写后立即失败），再验证 tag target commit 是当前
  source HEAD 祖先、`git show <target>:<Charter-Path>` 与工作树 bytes/hash 同 tag trailer 完全
  一致、工作树干净、`approved_code_tree_sha256` 与受控源码完全一致、以及所有
  CLI/runtime effective params 等于 charter。run metadata 记录 exact tag name/object ID、target commit、
  signer fingerprint/human ID、trust-root hash、charter path/hash、code-tree hash 和 source HEAD。
- 任何 raw-run/evidence commit 必须是 tag target 的后代。charter 一旦修改，必须以严格递增的
  `<N>` 创建新签名 tag；旧 tag/hash 下的全部 raw runs 自动 invalid。validator 禁止跨
  tag object、charter hash、signer/trust-root hash 或 source ancestry 聚合。重写/移动旧 tag 永久拒绝。
- `run_plan` 从 charter seed 生成稳定 shuffle；report 保存 seed 和 exact arm/task/run 顺序，不能在
  运行时临时取未记录随机数。
- task failure、timeout、oracle inconclusive 和缺失审计都是实验数据，不得从分母移除。中断的
  matrix 标记 incomplete，不生成 default recommendation。
- aggregate 除现有 resolved/token/time/failure 外，输出 helped/hurt、irrelevant injection、
  missing relevant、citation precision/recall、各分区选择/drop reason；raw run/artifact 引用包含
  hash，不提交私有正文。
- deterministic injection snapshot 补充 threshold/k/tie-break/blank/error 覆盖；它只能证明规则
  正确，不能代替 coding outcome。

报告 validator 要求五臂齐全、冻结字段一致、最低样本数符合批准 charter、没有被静默排除的失败，
且 raw references 可定位，并逐 run 验证 charter hash、批准 commit 祖先关系和 source HEAD。任何
invalidated run 都不能进入分母或 recommendation；缺臂时 matrix 只可标 incomplete。recommendation
只可在所有预注册预算通过时选择最小 k；否则固定输出 `keep_disabled`。生成报告本身不授权
runtime default、merge 或 release。

### 6. Hook footer 与持久化 CLI status

“状态”有两个明确 surface，不再混用：

- SessionStart hook footer 是本次请求即时状态，来自当前 `ContextRenderStats`，显示
  `off/applied/abstained/error`、三个受控分区 eligible/final-injected/drop reasons、total/delta
  truncation 和 audit write result。
- `remem status` 文本与 `remem status --json` 是持久化最近状态。扩展现有
  `latest_session_memory_spend`，从最近一个完整 `context_injection_items.injection_run_id` 聚合
  relevance state、policy/scorer version、threshold、k、final injected/drop counts、
  `latest_relevance_epoch` 和 `audit_completeness`，同时保留 chars/tokens/emit/suppress 现有字段。

status query 用 session/project/injection key/context hash 与 latest epochs 对齐 output/item evidence；
item run 必须原子写入。最近 context epoch 比最近完整 relevance run 新、item run 部分/缺失或审计写入
失败时，status 显示 `audit_incomplete`/`unavailable` 并保留两个 epoch，不回退展示更旧 run 为当前。
唯一例外是第 4 节定义的原子 suppression/reuse run：它与 context row 共用 event epoch、
精确引用 prior complete emitted run 并自包含本次零注入 item evidence。该情形显示
`relevance_state=suppressed_reused`、`audit_completeness=complete`、`final_injected=0` 和
`reused_from_run_id`，不显示 `applied`，也不把来源 run 的 injected count 携带到本次。
旧 DB 或还没有 relevance provenance 的 rows 显示 `unavailable_legacy`，不做 migration、不误报
`applied`。JSON 新字段是 additive；文本把它们加入现有 `Latest session memory footprint` block。

### 7. 文档、版本与交付边界

implementation 必须同步 README、Architecture、预算设计和 current-memory-contracts，说明三个
受控 view、Core 排除顺序、hook/footer 与 CLI status 区别、状态/reason、离线隐私、关闭方式和实验
门禁。runtime 变更按仓库规则同步
Cargo/plugin/release/npm 版本。spec PR 只含本 packet、使用 `Refs #854`；implementation 必须等
human 批准且 issue 进入 `ready_to_implement` 后另开 PR。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001` Core 先冻结且不参与 k | existing Core selector + relevance plan + render orchestration | `cargo test context::tests::relevance::freezes_baseline_core_before_governed_views -- --exact`；同 fixture 对比 off/enabled Core IDs/body/order 完全相同，并断言 scorer 输入与 k accounting 均无 Core ID |
| `B-002` off 兼容 | policy parser + render snapshots | `cargo test context::tests::relevance::off_mode_preserves_baseline_sections -- --exact`；golden 比较 section bodies/order/count，允许 footer 仅新增 `off` row |
| `B-003` 共同分数/门槛 | approved scorer adapter + Core-excluded views | `cargo test context::tests::relevance::all_selected_candidates_have_common_score_at_threshold -- --exact`；fixture 覆盖 Lessons/Sessions/Index view，并以高分 Core/duplicate 负例证明不进入 scorer |
| `B-004` 固定约束顺序/k 不虚耗 | selection plan + section/item-aware total budgets | `cargo test context::tests::relevance::applies_core_exclusion_dedupe_threshold_k_section_total_order -- --exact`；k=3 fixture 含 Core overlap 与跨-view duplicate，仍选择三个唯一非 Core 合格项 |
| `B-005` 不回填 | selection plan | `cargo test context::tests::relevance::does_not_backfill_below_threshold -- --exact` |
| `B-006` 确定性/tie-break | scorer + stable sort | `cargo test context::tests::relevance::same_snapshot_has_stable_scores_and_ties -- --exact`，不同插入/HashMap 顺序重复 100 次结果相同 |
| `B-007` 空/弱 query abstain | implicit query validator + render | `cargo test context::tests::relevance::weak_query_abstains_only_governed_sections -- --exact` |
| `B-008` 评分失败整体关闭 | scorer boundary + plan state | `cargo test context::tests::relevance::partial_or_invalid_scores_fail_closed -- --exact`；timeout/NaN/Inf/partial fixtures 均无受控 injected 项 |
| `B-009` 非法配置 | typed policy parser | `cargo test context::tests::relevance::invalid_enabled_config_is_visible_error -- --exact`，缺字段/阈值越界/k=0/未知 scorer 均拒绝 |
| `B-010` 完整 item/最终审计 | item-aware planner/truncation + gate stable keys + shared audit finalizer | `cargo test context::tests::relevance::audit_matches_complete_final_keys -- --exact`、`cargo test context::tests::truncation -- --nocapture`；total/delta cut 落在 item 中部时输出无半项且 DB 只有完整 survivor 为 injected；pre-render 空输出断言 injected=0；`cargo test context::prompt_submit::tests -- --nocapture` 证明 PromptSubmit 同 finalizer 不靠 title 子串且空/有输出审计不回归 |
| `B-011` hook footer 状态与 counts | stats/footer + render snapshots | `cargo test context::tests::relevance::footer_distinguishes_relevance_and_truncation -- --exact`；`render.rs`/`render_inline.rs` snapshots 覆盖四状态、完整 injected keys 与所有 closed reasons |
| `B-012` 审计失败可见 | audit call site + diagnostics/footer | `cargo test context::tests::relevance::audit_write_failure_is_error_and_not_complete -- --exact` |
| `B-013` 并发隔离 | request-private plan | `cargo test context::tests::relevance::concurrent_requests_do_not_share_selection_state -- --exact` |
| `B-014` 重试/策略身份 | plan determinism + data-version | `cargo test context::tests::relevance::retry_is_idempotent_and_policy_change_invalidates_gate -- --exact` |
| `B-015` 取消/部分结果 | scorer deadline + bench runner completion | `cargo test context::tests::relevance::cancellation_publishes_no_partial_plan -- --exact` 与 `cargo test eval::coding_bench::tests::interrupted_matrix_cannot_recommend -- --exact` |
| `B-016` immutable charter/五臂 matrix | coding-bench CLI/runner/signed-tag preflight/validator | `cargo test eval::coding_bench::tests::sessionstart_matrix_requires_trusted_signed_charter_tag -- --exact`；负例覆盖缺 tag、轻量/未签名 tag、签名坏、不可信 fingerprint/identity、repo-local 信任根、错 trailer/path/hash、tag target 非祖先、dirty bytes、code-tree/effective-param drift、tag 移动或重用、未批准 amendment 和跨 tag/hash runs，均在首个 raw row 前或 aggregate 前失败；CLI dry-run 输出 exact tag object/target/signer/trust-root/charter/code-tree/source HEAD |
| `B-017` 完整指标/证据 | coding-bench artifact/score validator | `cargo test eval::coding_bench::tests::sessionstart_report_requires_outcome_noise_cost_latency_and_raw_refs -- --exact`；删除任一字段的 fixture 必须失败 |
| `B-018` 最小合格 k/不启用 | report decision | `cargo test eval::coding_bench::tests::recommends_smallest_arm_within_all_budgets_or_disabled -- --exact`，含无 arm 合格负例 |
| `B-019` coding evidence 必需 | report validator | `cargo test eval::coding_bench::tests::golden_only_cannot_authorize_default -- --exact`；缺失/incomplete coding report 输出 `keep_disabled` |
| `B-020` 本地/隐私 | scorer boundary + report sanitizer | `cargo test context::tests::relevance::scorer_uses_no_external_io -- --exact`；人工检查报告/日志不含 fixture secret canary 或原文 |
| `B-021` 双 host/旧配置 | shared policy path + host snapshots | `cargo test context::tests::relevance::claude_and_codex_share_policy_and_old_config_is_off -- --exact`；隔离旧 DB 启动无需 migration |
| `B-022` human/evidence gates | workflow labels + implementation PR evidence | `python3 checks/route_gate.py --repo . --route implement --issue 854 --state ready_to_spec --json` 必须 blocked；人工确认 research/scorer/threshold/budgets/approval 缺一时无 tasks/default claim |
| `B-023` hook 与 CLI status 边界 | pre-render/store atomic reuse finalizer + `status_spend.rs` + status types/text/JSON + hook footer | `cargo test context::injection_gate::pre_render::tests::strict_pre_render_suppression_writes_atomic_reuse_evidence -- --exact`；源 complete run 时 context/item 同 epoch 且 status=`suppressed_reused`/complete/final_injected=0；identity mismatch、legacy/partial source 和注入式 item-write failure 时 transaction 回滚、epoch/suppress count 不前进且 render 被调用。再运行 `cargo test cli::actions::query::status::tests -- --nocapture` 与 `cargo test db::query::status_spend::tests -- --nocapture`；snapshots 断言 chars/runs 保留、原子 reuse 不误报 incomplete、真无 item run 仍 incomplete、legacy 仍 `unavailable_legacy` |

## 数据流

```text
ContextRequest + immutable ContextPolicy
  -> existing authorized DB load / implicit query
  -> existing Core selector freezes Core IDs/body (never scored, never counts toward k)
  -> unique candidates {Lessons, Sessions, MemoryIndex minus Core IDs}
  -> approved local scorer (all-or-nothing)
  -> threshold -> stable global k -> existing section budgets
  -> item-aware render/total/duplicate/delta gate returns complete surviving stable keys
  -> normal emit: hook footer + atomic finalized context_injection_items
  -> strict pre-render hit: atomic suppression epoch + zero-injected reuse item run
  -> remem status text/JSON reads latest complete relevance run beside existing chars/runs

immutable charter commit/blob + exact trusted signed annotated approval tag + external trust root
  -> runner preflight verifies tag signature/identity + target ancestry + bytes/hash + code tree/effective params
  -> production baseline, k1, k3, k5, k10 arms
  -> raw hashed run evidence bound to charter hash/source HEAD (failures included)
  -> validated aggregate metrics/budgets
  -> smallest passing k OR keep_disabled
  -> separate human default approval
```

持久化只复用现有 `context_injection_items` 与 `context_injections`；Session summary ID 来自现有
`session_summaries.id`，status 以现有 run/key/hash/epoch evidence 聚合，不新增表/列或 migration。
普通运行不进行外部调用。benchmark 使用隔离的 HOME/REMEM_DATA_DIR、
已批准 fixture 和固定 runner；提交的 aggregate/report 只含去敏元数据、metrics 和 artifact hash。

## 备选方案

- **只缩小现有字符/item budgets**：拒绝。不能保证留下的内容与任务相关，也无法解释为何丢弃。
- **三个分区各自 threshold 后各取 k**：拒绝。无法形成 issue 要求的总体少量注入，也会让总量
  最多达到 3k；跨分区不可比仍未解决。
- **先把所有 memory 放进 global k 再在 renderer 排除 Core/duplicates**：拒绝。Core 或重复项会
  虚耗 k，产生“k=3 但只注入 1 个受控项”的错误实验语义；必须先冻结/排除/去重。
- **总字符裁剪后用 title 搜索推断存活 item**：拒绝。重复标题和半截多行 item 会使审计与输出
  不一致；预算或 finalizer 必须携带 stable keys。
- **pre-render suppression 只复用旧 status/时间戳**：拒绝。旧 run 可能是 legacy、partial 或
  来自不同 policy identity；只有同事务的自包含 reuse item run 能证明本次零输出为正常 suppression。
- **在 charter JSON 内写 `approved_by` 或自身 commit SHA**：拒绝。候选 branch 可自我声明批准，
  而内容不可能预先包含承载它的 Git commit SHA；批准必须由独立签名 tag + checkout 外信任根建立。
- **直接使用 PromptSubmit token overlap**：暂不采用。该门槛服务不同 hook/查询，缺少三个分区
  的共同校准；可在 approval 证据证明后作为候选 scorer，而不是隐式默认。
- **直接复用 hybrid RRF 或 GH-851 reranker**：暂不采用。当前 RRF 分数被丢弃且只覆盖 memory，
  其他 issue 未合并/未校准的产物不能成为本 spec 的依赖。
- **评分失败时回到旧注入**：拒绝。会把“无法判断”伪装成成功并重新注入噪声；受控分区必须
  fail-closed，非受控分区保持输出。
- **只跑 golden/injection snapshot**：拒绝。它能测规则，却不能证明真实 coding outcome。
- **另建 k-sweep runner**：拒绝。现有 coding bench 已具备隔离、失败分类、token/time 和记忆
  attribution；扩展同一证据链更容易校验。
- **立即默认启用 issue 建议数字**：拒绝。研究报告缺失，评分/预算/样本均未获批准。

## 风险

- Security: 候选正文和 implicit query 可能含私密信息。评分保持本地且不记录原文；benchmark
  只用批准 fixture/隔离数据。涉及日志、report sanitizer 的实现必须人工 security review。
- Compatibility: footer/audit 增加状态字段是可见格式扩展；section body 在 off 模式保持基线。
  scorer identity 纳入 data-version 会让策略变化后的首次请求重新注入，这是正确失效行为。
- Correctness: 三分区分数若未校准，global k 会系统性偏向某一分区；因此 calibration evidence 是
  硬 blocker。Core 必须在相同 reference time 下先冻结，total/delta 后只信任完整 stable keys；
  title 子串、半截 item 或在 k 后排除 Core 都会污染 runtime 与 eval 结果。pre-render
  suppression 若只推进 output epoch 会误报 audit incomplete；若照搬旧 injected 状态则会误报本次实际输出。
- Performance: 对全部候选评分增加 SessionStart CPU/latency。期限和候选上限必须预先批准；超时
  不能部分输出。sweep 报告 wall time 和 input size。
- Evaluation validity: 任务集过小、执行顺序、model 或可执行代码漂移、伪造 human approval
  或事后挑指标会污染结论。签名 tag target 必须是每个 raw-run source/evidence commit 的祖先；
  signer/trust-root/charter/code-tree/hash/amendment 变化废止旧 runs，五臂同环境、失败计入分母和预注册
  budgets 防止选择性报告。
- Maintenance: scorer/version/policy/data-version/report schema 必须同步演进；closed reason 改义或
  新增受控分区需新 spec revision，不能仅改实现。
- Data: 不做 schema migration；若实现中发现现有 audit 列无法无损表达批准契约，必须暂停并修订
  tech spec，不能静默拼接不可解析字符串或临时加列。

## 测试计划

- [ ] Approval precondition：补齐/撤销研究报告引用，批准 scorer/calibration、threshold、弱查询、
      deadline、sample size、environment、budgets 和默认决策规则；记录 spec approval evidence。
- [ ] Focused policy/scorer/selection tests：执行 Product-to-Test Mapping 中 `context::tests::relevance`
      全部 exact tests，覆盖三个分区、空、非法、错误、并发、重试、取消和稳定排序。
- [ ] Render/audit integration：验证 actual output keys、footer stats 与 DB item records 一致，覆盖
      Core exclusion/k accounting、section budget、item-aware total truncation、duplicate suppression、
      delta preview 和 PromptSubmit 共享 finalizer；执行 `render.rs`、`render_inline.rs`、
      `truncation.rs`、`gate_pipeline.rs` 与 PromptSubmit focused tests。
- [ ] Pre-render reuse integration：执行 `src/context/injection_gate/pre_render.rs` 内联 tests 与
      `src/context/injection_gate/tests.rs`；覆盖 exact complete source、identity/policy/hash mismatch、legacy/
      partial source、reuse-of-reuse 拒绝、同秒多次 suppression 的唯一 run ID，以及 item/context
      任一写故障的 transaction rollback + full-render fallback。
- [ ] Eval unit tests：执行 mapping 中 `eval::coding_bench::tests::sessionstart_*` tests，负例覆盖
      缺 arm、字段漂移、失败剔除、incomplete report、golden-only recommendation，以及 B-016 的
      signed-tag/trust-root/ancestry/charter/code-tree/effective-parameter 全部负例。
- [ ] `cargo test context::tests -- --nocapture`。
- [ ] `cargo test eval::coding_bench -- --nocapture`。
- [ ] `python3 scripts/ci/check_plugin_version_sync.py`。
- [ ] `cargo fmt --check`。
- [ ] `cargo check`。
- [ ] `cargo clippy -- -D warnings`。
- [ ] `cargo test`。
- [ ] 以 exact signed approval tag object 与 checkout 外 trust root 运行完整五臂 coding-bench，validator
      通过并生成去敏报告；人工核对 raw artifact hashes、tag object/target/signer/trust-root hash、
      charter/code-tree hash、每 run source HEAD、失败分母、配置身份和 recommendation，不把 dry-run 当结果；
      修改 charter 或 signer/trust root 后确认旧 runs 全部 invalid。
- [ ] `cargo test cli::actions::query::status::tests -- --nocapture` 与
      `cargo test db::query::status_spend::tests -- --nocapture`，核对文本/JSON status 的 current、
      suppressed_reused、incomplete、legacy 四类 fixture。
- [ ] `cargo run -- eval-extraction --json --check-baseline` 与
      `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`，确认共享 runtime 无回归。
- [ ] `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`，PR body
      使用 `Closes #854` 仅限 implementation PR；spec PR 使用 `Refs #854`。
- [ ] `git diff --check origin/main...HEAD`，人工确认没有网络/LLM scoring、schema migration、私密正文
      artifact、Core/Preferences/Workstreams 行为漂移或未在 manifest 中声明的文件。

## 回滚方案

未合并时关闭 implementation PR；spec 和实验报告保留为审计证据。若实现合并但尚未默认启用，
立即把 policy mode 保持/恢复为 `off`，再通过独立回滚 PR 回滚 runtime、docs 和版本变更；不删除
已写入的 append-only audit rows。旧数据无需降级或 migration rollback。

若未来默认启用后出现任务质量、隐私、延迟或审计回归，先以已文档化配置 fail-safe 关闭策略，
再回滚默认变更；不得在 scorer 失败时切成无提示旧注入。任何重新启用都需要新的完整 sweep、CI、
review、human merge/release 授权。PoC、spec、implementation、default、merge 和 release 授权互不
继承。

<!-- specrail-planned-changes
{"version":1,"issue":854,"complete":true,"paths":["Cargo.lock","Cargo.toml","README.md","docs/ARCHITECTURE.md","docs/context-budget-design-2026-04-29.md","docs/specs/current-memory-contracts/PRODUCT.md","docs/specs/current-memory-contracts/TECH.md","eval/coding-bench/README.md","eval/coding-bench/fixtures/tasks.json","eval/coding-bench/reports/issue854-sessionstart-k-sweep.json","eval/coding-bench/sessionstart-k-sweep-charter.json","npm/remem/package.json","plugins/remem/.codex-plugin/plugin.json","plugins/remem/runtimes/remem-releases.json","specs/GH854/product.md","specs/GH854/tech.md","src/cli/actions/eval.rs","src/cli/actions/query/status.rs","src/cli/actions/query/status/tests.rs","src/cli/actions/query/status/types.rs","src/cli/eval_types.rs","src/cli/tests_eval.rs","src/context.rs","src/context/audit.rs","src/context/implicit_query.rs","src/context/injection_gate.rs","src/context/injection_gate/data_version_hint.rs","src/context/injection_gate/delta.rs","src/context/injection_gate/pre_render.rs","src/context/injection_gate/store.rs","src/context/injection_gate/tests.rs","src/context/policy.rs","src/context/prompt_submit.rs","src/context/query.rs","src/context/relevance.rs","src/context/render.rs","src/context/render/eval.rs","src/context/render/stats.rs","src/context/render/truncation.rs","src/context/sections/index.rs","src/context/sections/lessons.rs","src/context/sections/sessions.rs","src/context/style.rs","src/context/tests/diagnostics.rs","src/context/tests/gate_pipeline.rs","src/context/tests/mod.rs","src/context/tests/ownership.rs","src/context/tests/relevance.rs","src/context/tests/render.rs","src/context/tests/render_inline.rs","src/context/tests/render_stability.rs","src/context/tests/truncation.rs","src/context/types.rs","src/db/query/status_spend.rs","src/eval/coding_bench/artifact.rs","src/eval/coding_bench/condition.rs","src/eval/coding_bench/fixture.rs","src/eval/coding_bench/run_plan.rs","src/eval/coding_bench/runner.rs","src/eval/coding_bench/score.rs","src/eval/coding_bench/tests.rs","src/eval/coding_bench/types.rs"],"spec_refs":["specs/GH854/product.md","specs/GH854/tech.md"]}
-->

本文件不授权实现。只有 human 完成上述 blockers、批准修订后的 product/tech spec 并把 GH-854
置于 `ready_to_implement` 后，才能生成 tasks 和 implementation PR。
