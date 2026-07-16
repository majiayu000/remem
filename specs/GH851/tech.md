# Tech Spec

## Linked Issue

GH-851（Epic: GH-849）

## Product Spec

[`product.md`](product.md)

## Codebase Context

以下事实基于 `origin/main` `778a336999876817a268c546aaa2bc6f3e3524ae`。

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| 标准记忆检索 | `src/retrieval/search/memory/text.rs`, `src/retrieval/search/memory/weights.rs`, `src/retrieval/search/memory/explain.rs` | 按查询条件运行 FTS、entity、fact、vector、temporal 等候选通道，以 weighted RRF 融合，再加载记忆、执行 source-anchor demotion/标签和分页；通道是条件化的，并非每次固定启用四个 | rerank 必须插在资格与现有排序完成之后、分页之前，并保留 explain/timing |
| 公共 search 入口 | `src/cli/actions/query/search.rs`, `src/memory/service/search.rs` | CLI 构造 service request，service 调用 retrieval search；API/MCP 的标准检索复用 service 边界 | 标准调用方应自动得到同一 rerank 行为，不能只修改 CLI |
| SessionStart 检索 | `src/context/query.rs`, `src/context/implicit_query.rs`, `src/context/hybrid_context.rs` | 从项目、分支、提交、summary/workstream 生成隐式 query；context 模块自行运行候选通道与 weighted RRF，再按 limit 取结果 | 当前与标准 search 重复 RRF 实现，尚无共享二阶段路径 |
| Timing | `src/perf.rs`, `src/retrieval/search/memory/text.rs`, `src/context/render.rs`, `src/context/render/timer.rs` | search 记录各 phase 并在 explain 中暴露；context 记录整体 render phase。golden eval 报告 retrieval p50/p95，但当前文档将 latency 作为 trend 而非硬 gate | 需要共享 rerank phases，并补充独立冷/热 SessionStart p95 证据；不能声称 main 已有数字预算 |
| 本地模型管理 | `src/retrieval/embedding.rs`, `src/retrieval/embedding/local_semantic.rs`, `src/cli/actions/embedding.rs`, `src/cli/embedding_types.rs` | local-onnx embedding 通过显式 download 物化 fastembed 模型，写入带文件/hash/bytes/runtime/source 的 manifest；运行时先验证本地 inventory | reranker 应复用安装与校验模式，但拥有独立 kind/preset，不能把 embedding manifest 当作 reranker 证据 |
| ONNX runtime | `Cargo.toml`, fastembed `5.17.2` local-onnx feature | 当前依赖已提供本地 text embedding；该版本 API 也提供 text rerank 和从用户定义本地 ONNX/tokenizer 构建 reranker 的入口 | hook/runtime 应只从已验证文件构建，避免会访问模型 hub 的初始化路径 |
| Doctor | `src/doctor/embedding.rs` | 检查 embedding provider 可用性与覆盖；没有 reranker inventory、hash 或 runtime 状态 | 必须新增独立可诊断状态，并区分 off 与 enabled-but-broken |
| Golden eval | `eval/golden.json`, `eval/README.md`, `eval/gates/baseline.json`, `src/eval/` | 数据集包含 paraphrase、associative 等 slices，指标含 H@k、MRR@10；现有 gate 可检查回归，但没有 rerank-off/on 配对报告或 rerank promote gate | 质量批准必须来自固定本地 A/B artifact，不外推论文数字 |

## 设计方案

### 1. 在资格过滤后建立共享 rerank stage

新增一个 retrieval-owned 的共享模块（计划位置 `src/retrieval/rerank/`），只接收已经通过调用方现有
资格规则的有序候选，不拥有候选召回、权限过滤或数据库查询。建议的内部合同为：

```rust
struct RerankRequest<'a> {
    query: &'a str,
    candidates: &'a [EligibleCandidate],
    top_n: usize,
    top_k: usize,
    page_offset: usize,
    deadline: Option<Instant>,
}

struct RerankOutcome {
    ordered_ids: Vec<String>,
    status: RerankStatus,
    model_manifest_sha256: Option<String>,
    input_count: usize,
    output_count: usize,
    timings: Vec<PhaseTiming>,
}

enum RerankStatus {
    Applied,
    NotApplied { disabled_reason: RerankDisabledReason },
}
```

实际公开类型与字段名可按现有模块边界调整，但禁止使用 `Any` 或自由字符串代替 closed enum。
`RerankDisabledReason` 至少区分：`off`、`empty_query`、`insufficient_candidates`、
`model_missing`、`model_corrupt`、`model_load_failed`、`inference_failed`、`deadline_exceeded`、
`cancelled` 和 `pagination_window_exceeded`。序列化 API 边界使用既有 camelCase 规则，Rust 内部保持
snake_case。

标准 search 在 RRF、memory load、现有 eligibility/source-anchor 处理之后且分页之前调用该 stage；
SessionStart 在自己的项目/branch/owner/suppression 过滤及 RRF 后调用同一 stage。两者保留不同的
一阶段候选信号，但必须复用：

- 同一个 query normalization 边界；
- 同一个 `EligibleCandidate -> RerankDocument` 投影函数；
- 同一个 top-N 截取、model scoring、tie-break、回退和 timing 实现；
- 同一个 `RerankDisabledReason` 与诊断序列化。

候选投影只包含 human-approved 的本地字段，建议从记忆标题、正文和类型形成有标签的确定性文本。
字段顺序、每字段及总 UTF-8 byte/token 上限必须在实现前用本地 eval 冻结；截断必须在 Unicode 边界
完成。project、owner 等授权属性只用于进入 stage 前的过滤，不把无关身份元数据送入模型。

### 2. top-N、top-k、稳定性与分页

RRF 的完整有序 eligible list 是唯一候选来源。stage 取最多 top-N，对每个候选计算 query-document
分数，按以下稳定键排序后返回所需 top-k/page：

1. rerank score 降序；
2. 原 RRF rank 升序；
3. stable memory id 升序。

任何 NaN/非有限分数都视为整次 inference failure，不发布部分排序。top-N/top-k 必须是有界、
启动时可验证的配置，`N >= k > 0`。具体数值不在本 draft 猜测，由质量/性能 A/B 后人工冻结。

对分页请求，rerank candidate window 必须至少覆盖 `offset + limit + 1` 才能计算稳定的页面和
`has_more`。若该窗口超过批准的 top-N/max-window，不混合“已 rerank 前缀 + 未 rerank 尾部”，而是
整次使用基线 RRF 并返回 `pagination_window_exceeded`。这保留跨页确定性并避免重复/漏项。

### 3. 本地模型 inventory 与显式下载

复用 local semantic embedding 的“显式下载 -> staging -> 文件/hash/bytes 校验 -> 原子 manifest
发布 -> 运行时只读验证”模式，但将 reranker 作为独立模型 kind 和 inventory，避免覆盖 embedding
状态。计划提供与现有命令风格一致的显式 surface，例如：

```text
remem reranker download --preset <approved-preset>
remem reranker status --json
```

最终命令名属于实现 review，但必须满足同一行为合同。manifest 至少固定 schema version、preset、
上游 model id/revision、fastembed/runtime version、每个本地文件的相对路径/bytes/SHA-256，以及整个
manifest 的稳定 hash。下载在临时目录完成并校验；只有原子 rename 后才可被运行时发现。升级失败时
保留上一份已验证版本。

当前 fastembed 候选包含多个 reranker preset，但本 spec 不选型。实现前必须在 remem golden eval
和批准的本地性能 profile 上比较候选，然后由 maintainer 冻结 exact preset/model revision/file
hash。不能以模型卡或论文指标代替该证据。

运行时不得调用可能访问 Hugging Face/model hub 的 `TextRerank::try_new` 类路径；只允许用已校验
的 ONNX/tokenizer bytes 走本地 user-defined 初始化。网络 deny 测试必须覆盖 CLI search、service
search、API/MCP 与 SessionStart。

### 4. 加载、并发、deadline 与原子回退

每进程用明确状态机缓存已验证 manifest 对应的模型实例：`Uninitialized -> Loading -> Ready`，
加载失败产生可观察 error，不发布半初始化实例。模型版本以 manifest hash 为缓存 key；显式下载
原子切换后，新请求可重新加载，正在运行的请求继续持有旧的只读版本，不混用文件。

fastembed reranker 的可变/线程安全边界必须由类型验证决定，不假设 `Sync`。如推理 API 需要可变
访问，用有界 worker 或 mutex 串行保护模型实例，但候选、输出和 deadline 都是请求私有；不得用
全局可变 buffer。锁中毒/worker 退出必须返回 `model_load_failed` 或 `inference_failed`，不能 unwrap
或静默重建未验证模型。

同步 ONNX 调用未必能在单次 kernel 中硬中断，因此 deadline/cancellation 至少在排队、每个有界
batch 前后检查。只在所有候选成功评分且请求仍有效后原子构造新顺序；任何 batch 失败、deadline
越界或 cancellation 都丢弃临时分数。调用方仍等待结果时返回完整 RRF baseline + reason；调用方
本身取消时遵循现有取消返回，不执行 DB 写入、下载或后台无限重试。

### 5. Fail-visible 状态与 doctor

`SearchExplain` 增加独立 `rerank` stage，而不是把 reranker 伪装成召回 channel。该 stage 包含
requested/applied、preset、manifest hash、top-N/top-k、输入/输出数、stable disabled_reason 和
分阶段 timings。非 explain 的 service/API response 必须有兼容的结构化诊断位置；如果现有公开
response 无法安全加字段，必须通过已存在的 diagnostics envelope/版本化扩展提供，不能只写 debug
日志。

SessionStart 将同一 outcome 写入 context render stats/audit 和 error 日志，但不把故障字符串拼入
记忆正文。日志不得包含完整 query/记忆内容或敏感模型输入。

doctor/status 增加 reranker 检查矩阵：

| Config / inventory | Runtime behavior | Doctor |
| --- | --- | --- |
| off | baseline RRF，reason `off` | OK，明确 disabled |
| enabled + manifest/files/hash valid | 可加载并 rerank | OK |
| enabled + missing manifest/file | baseline RRF，error，reason `model_missing` | Fail |
| enabled + size/hash/manifest invalid | baseline RRF，error，reason `model_corrupt` | Fail |
| enabled + local runtime load failure | baseline RRF，error，reason `model_load_failed` | Fail/可复现诊断 |

动态单请求 inference failure 通过 response/context outcome 和 error 日志暴露；若要跨进程持久化
“最近错误”，必须另行批准数据保留与并发合同，本变更不为诊断随意新增数据库状态。

### 6. Timing、p95 与质量 promote gate

共享 stage 使用现有 `PhaseTiming` 记录至少：

- `rerank_model_load`（首次/版本切换时单列，不能摊入所有 warm 请求）；
- `rerank_queue_wait`（存在共享 worker/mutex 时）；
- `rerank_inference`；
- `rerank_total`。

标准 search explain/log 和 SessionStart render stats 使用相同名称与测量点。benchmark 报告必须把
cold（新进程、模型尚未加载）与 warm（已加载）分开，记录机器/OS/CPU、thread 数、数据集 hash、
manifest hash、top-N/top-k、候选输入长度分布、样本数、p50/p95/max 和失败数。

`origin/main` 目前没有数字化的 SessionStart p95 硬门槛，golden eval 的 latency 也只作为趋势。
因此以下内容是 **spec approval 与默认启用阻塞项**：maintainer 必须批准参考 profile、最小样本数、
cold/warm 策略、warm rerank p95 增量预算、cold SessionStart p95 总预算及超预算处置。没有该批准时
配置保持 off，PR 不得以论文或开发机单次 timing 宣称满足性能目标。

质量评估新增可复现的 rerank A/B 模式：同一 commit、`eval/golden.json` hash、fixture、模型
manifest、top-N/top-k、输入上限和运行环境，先跑 off 再跑 on，产出一个同时保存双方原始 metrics
和 delta 的 JSON artifact。预登记主指标为 paraphrase+associative 合并集合的 MRR@10 或 Hit@5
之一，必须在运行前写入配置/报告 header；四个单 slice 指标全部 non-regression，两个合并指标也
non-regression，主指标 delta 必须 `>= 0.05`。现有其他 gated slices/overall metrics 继续执行仓库
已有 non-regression gate，不能为 rerank 放宽阈值。

### 7. 配置、启用与回滚边界

配置至少包含 enabled、approved preset、top-N、top-k、输入上限和请求 deadline；默认 `enabled=false`
直到所有阻塞项批准。非法组合在配置/启动边界 fail-visible，不在请求中偷偷改成默认值。

关闭分支必须完全绕过模型 load/inference，并调用现有 RRF 排序与分页代码，从而在固定 fixture 上
保持 byte/order-equivalent baseline。该设计不需要数据库 schema migration；模型是数据目录中的
可删除 cache/asset。若质量、延迟、兼容性或错误率回归，先关闭同一开关恢复 search 与 SessionStart，
再通过独立 issue/PR 处理根因。

### 8. 批准前阻塞项

下列事实在当前 `main` 中不存在，不能由实现者自行推断：

1. exact reranker preset、上游 revision、文件集合和 manifest hash；
2. top-N/top-k、candidate document 字段与 UTF-8/token 上限；
3. SessionStart 的参考机器、样本数、cold/warm 测量政策和数字化 p95 预算；
4. rerank-off/on 配对 eval artifact、预登记主指标及 >=0.05 本地提升证据；
5. search 与 SessionStart 共享 rerank stage、结构化 diagnostics、reranker doctor/inventory surface。

1-4 必须由本地 PoC/eval 产出并经 maintainer 批准；5 是后续 implementation 的工作范围。阻塞项
未关闭前，本文件保持 Draft，不创建 `tasks.md`，不进入 `ready_to_implement`。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001`–`B-003` | shared rerank stage + standard search/context call sites | fixture 中 top-N 外、跨项目、owner 不匹配、suppressed/stale 候选永不出现；相同输入稳定排序 |
| `B-004`, `B-014` | reranker download/inventory + local-only loader | network-deny integration tests；显式下载 staging/hash/atomic publish；离线已安装模型成功 |
| `B-005`, `B-015` | config bypass + baseline path | off/on-disabled fixture 对比完整 ids/order/pagination/has_more；关闭不触发 loader |
| `B-006`, `B-011` | manifest verifier + explain/context diagnostics + doctor | missing manifest/file、wrong size/hash、corrupt ONNX；baseline 返回、stable reason、error log、doctor Fail |
| `B-007`, `B-008` | worker/model cache/request transaction | load/inference fault injection、deadline/cancel、并发初始化与请求隔离；无部分顺序、无 panic/污染 |
| `B-009` | page window validator | exact max window、one-over、多个连续页面；one-over 完整 RRF fallback 且无重复/漏项 |
| `B-010` | canonical document projection + shared outcome | 同一 query/candidate fixture 经 search/SessionStart 得到相同 model inputs、scores、tie-break/reason/timings |
| `B-012` | eval A/B runner/gate | 固定 hash artifact；单 slices 与合并 MRR@10/Hit@5 non-regression；预登记主指标 delta >=0.05 |
| `B-013` | PhaseTiming + SessionStart benchmark | cold/warm 分离，批准的样本/profile 下 p95 gate；缺少批准预算时 enablement gate 失败 |
| `B-016` | SpecRail/PR gates | 无 human-approved blockers evidence 时不能标记 ready_to_implement/default-on/merge-ready |

## 数据流

查询运行时不新增网络或持久化：

```text
explicit query / SessionStart implicit query
  -> existing candidate channels and RRF
  -> existing project/branch/owner/suppression/staleness eligibility
  -> ordered eligible top-N
  -> shared local rerank stage
       -> verify configured manifest/files (no network)
       -> canonical bounded query-document projection
       -> local ONNX scoring into request-private temporary buffer
       -> stable sort; publish only after complete success
  -> existing pagination/top-k/rendering
  -> structured rerank outcome + PhaseTiming

failure/cancel/deadline
  -> discard every temporary score
  -> complete pre-rerank RRF order (unless caller cancelled whole request)
  -> stable disabled_reason + error-level evidence
```

唯一新增的磁盘写入来自显式下载命令：

```text
user invokes download
  -> fetch selected approved preset into staging
  -> verify relative paths, sizes, SHA-256 and runtime metadata
  -> write manifest
  -> atomic publish under data_dir/models/reranker/<manifest-hash>
```

hook、search、API/MCP 和 doctor 不下载。doctor 只读 manifest/files；eval 只写用户指定的报告 artifact，
不修改运行数据库。

## 备选方案

- **用更高 RRF 权重或只增加向量 top-k**：拒绝。仍是独立通道分数融合，不能对完整 query-document
  对做二阶段相关性判断。
- **改用远程 rerank API**：拒绝。引入隐私、网络可用性、延迟和 hook 失败面，不满足本地合同。
- **在 SessionStart 首次运行时自动下载**：拒绝。会让 hook latency/离线行为不可控，并形成静默网络
  副作用。
- **search/context 各写一份 rerank**：拒绝。模型输入、错误回退和 timing 会漂移，违反共享路径要求。
- **缺模型时只记录 warning 并继续 RRF**：拒绝。用户会误以为配置已生效；必须 fail-visible reason、
  error 日志和 doctor failure。
- **部分候选评分成功后保留部分新顺序**：拒绝。结果不可解释且重试可能变序；整次原子回退。
- **直接采用某篇论文的模型与 +5pt 数字**：拒绝。数据分布、候选生成和硬件不同，只接受 remem 本地
  A/B artifact。
- **本 spec 直接填一个 p95 数字**：拒绝。main 没有权威预算，必须先由参考 profile 数据和
  maintainer 决策冻结。

## 风险

- Security: 模型与 tokenizer 是外部二进制资产；只接受固定 revision/hash、相对路径和原子
  inventory，防止路径穿越与部分文件加载。查询/候选不离开本机，日志不得记录原文。
- Compatibility: rerank 会有意改变启用状态下的结果顺序；off 路径必须保持原 RRF 和分页。新增公开
  diagnostics 字段需遵守 API schema/version 兼容策略，不能破坏旧客户端反序列化。
- Performance: cross-encoder 是同步 CPU 密集型工作，可能阻塞 hook 或服务请求。以有界候选、输入、
  worker/并发、deadline 和批准的冷/热 p95 gate 控制；预算缺失时保持 off。
- Maintenance: fastembed preset/API 与上游模型文件可能漂移；manifest 固定 revision/runtime/hash，
  升级必须重新走质量与性能 gate，不自动追随 latest。
- Reliability: 两套现有一阶段 RRF 可能继续演进；共享边界必须只接收 eligible candidates，并由
  cross-entry fixture 防止 rerank 语义分叉。
- Test integrity: 不允许通过降低已有 eval thresholds、删除 slice 或更换看到结果后的主指标来获得
  绿色；A/B artifact 必须记录配置和 hash。

## 测试计划

- [ ] Unit tests: manifest/path/hash verifier、document projection/UTF-8 boundaries、top-N/top-k config、
      stable tie-break、NaN、reason enum、page-window exact/one-over、atomic outcome。
- [ ] Model tests: 使用固定小型本地 fixture 验证 load/score；missing/corrupt/wrong hash/runtime
      mismatch 均不调用网络并完整回退。
- [ ] Concurrency tests: 多请求首次加载、加载失败、worker crash/lock poisoning、并发不同 query、
      cancellation/deadline at queue/batch/commit boundaries，无部分/跨请求污染。
- [ ] Search integration: 标准 CLI/service/API/MCP fixture 验证资格过滤、top-N membership、分页、
      explain diagnostics、off baseline parity。
- [ ] SessionStart integration: 相同 query/candidates 与 standard search 得到相同 canonical inputs、
      scores、order/reason；诊断/timing 可见且模型正文不含故障文本。
- [ ] Offline tests: deny all network；已安装模型正常，缺失/损坏返回 reason/error，hook 进程无下载
      文件、无后台任务。
- [ ] Doctor/status tests: off=OK，verified=OK，enabled missing/corrupt/load-failed=Fail，JSON/text 都不
      仅依赖颜色表达状态。
- [ ] Golden A/B: `eval/golden.json` 固定 hash，off/on 配对报告；paraphrase/associative 单 slice 和
      合并 MRR@10/Hit@5 non-regression，预登记主指标 `>= 0.05` absolute delta，其他既有 gate 不放宽。
- [ ] Performance: human-approved profile 上分别运行 cold/warm SessionStart benchmark，报告 rerank
      phases 与总 p50/p95/max/failures；预算未冻结或超预算时 gate fail。
- [ ] Compatibility: `local-onnx` on/off feature matrix、旧配置、无模型、现有 search/context golden
      fixtures；rerank off 的 ids/order/pagination/has_more 与 baseline 相同。
- [ ] Repository gates: `cargo fmt --check`、`cargo check`、`cargo test`、`cargo clippy -- -D warnings`、
      plugin/version sync checks、eval gates 和完整 `check_pr_preflight.py`。
- [ ] SpecRail gates: implementation 前重新运行 route gate；PR merge-ready 前以当前 head evidence 运行
      workflow、spec-vs-implementation、reviewThreads、CI 与 PR gate，保留 human merge authorization。

## 回滚方案

实现前无需回滚；关闭或不批准本 draft 即可。实现后首先把 reranker `enabled=false`，search 与
SessionStart 必须立即共同恢复现有 RRF 路径，不依赖数据库 migration 或删除模型。已下载模型可保留
以便诊断，也可在显式用户动作下删除；不得由 hook 自动清理。

若 implementation PR 尚未合并，关闭该 PR；若已合并且仅关闭开关不足，整体 revert implementation
commit 及其版本同步，不回滚/改写历史 spec。模型 manifest/schema 的后续升级必须继续支持关闭旧
版本或显式失败，不能自动加载未验证资产。

本文件只定义技术 draft，不是 `spec_approval`。Refs #851 / #849；只有 maintainer 关闭第 8 节的
证据阻塞并将 issue 置于 `ready_to_implement` 后，才可创建 `tasks.md` 或开始实现。最终 review、
merge、release 和默认启用仍是 human gates。
