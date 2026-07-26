# Product Spec

## Linked Issue

GH-934

complexity: large

## 用户问题

remem 已有 FTS、vector、entity、temporal、graph、generated enrichment 与 rerank 等检索能力，
但普通搜索和高风险修改仍主要依赖一套静态 channel/fusion 路径。用户无法可靠表达“恢复工作”、
“解释决策”、“调试失败”、“应用偏好”、“审查改动”或“探索历史”这六种不同证据需求，也无法
审计一次检索是否真的执行了对应的 trust、freshness、budget、degradation 与 abstention 策略。

原 PR #940 已实现 GH-934 的 Phase A：

- versioned `ContextIntent` / `RetrievalPlan` DTO；
- 六种 intent 的确定性 plan 编译、保守 fallback 与 stable plan hash；
- high-risk plan 收紧、generated enrichment contribution cap 与 rerank policy 占位；
- `remem context-plan` debug JSON。

Phase A 只编译 plan，不读取数据库，也没有把 plan 接入真实 retrieval、MCP/REST search、
`ContextBundle`/`ContextAudit` 或 benchmark artifact。Issue 的执行、评测与默认启用验收因此仍未
完成；PR #940 必须保持 `Refs #934`，不能以 Phase A 合并直接关闭 GH-934。

## 目标

- 把 versioned `RetrievalPlan` 接入真实 retrieval/rerank/Context Bundle 执行链路，并让执行结果可审计。
- 让 MCP 与 REST caller 能显式提供 intent、role、risk、scope 与 budget；显式值优先且不得被关键词
  推断覆盖。
- 对每个 channel 强制执行 candidate limit、weight、trust、validity、max contribution、timeout 与
  degradation；generated enrichment 永远是独立、可归因且受限的 signal。
- 建立六个 intent 的独立离线 golden slice、corrupted enrichment 对抗测试与
  `static_fusion` vs `intent_router` ablation。
- 只有在质量、memory-hurt、unknown fallback 与 p95 latency 门禁全部通过后才把 router 设为默认；
  门禁前保留兼容的静态默认与显式 opt-in。

## 非目标

- 不实现 HyDE、自由形式 query rewrite、新 embedding/provider 或实时多 agent 调度。
- 不替代 GH-850/GH-928 的 enrichment worker、GH-851 的 rerank 模型/执行器、GH-853 的 graph
  expansion 或 GH-854 的 SessionStart budget gate。
- 不允许 LLM 直接生成最终 plan、扩大 scope、降低 trust、绕过 abstention 或新增前台必需网络调用。
- 不重写 canonical memory、current-state、suppression、staleness 或 source-anchor 语义。
- 不在 spec 阶段修改生产代码、测试、版本 surface、PR base 或 GitHub 状态。

## Behavior Invariants

1. `B-001`：`ContextIntent` 与 `RetrievalPlan` 必须是 versioned、snake_case、可序列化且可重复计算的
   契约。六种 routable intent 必须保持 `resume_work`、`explain_decision`、`debug_failure`、
   `apply_preference`、`review_change`、`explore_history`；未知或无法分类输入保守落到
   `explore_history`，不得扩大 scope、降低 trust 或关闭 abstention。
2. `B-002`：同一合法 `ContextRequest`、显式 intent 与 policy version 必须产生 byte-stable plan
   JSON 和相同 SHA-256 `plan_hash`。plan 编译不得读取 clock、环境变量、数据库、网络、随机数或调用
   LLM；非法 schema、空 project、零 budget 或未知 enum 必须返回明确错误而不是生成默认成功 plan。
3. `B-003`：真实执行必须消费已验证的 `RetrievalPlan`，且每个 enabled channel 的候选来源、limit、
   weight、trust、validity、max contribution、timeout 与 degradation 都由该 plan 决定。执行器不得
   在 plan 外静默重新启用 channel、恢复静态权重或绕过 filter；plan validation 失败必须产生
   `blocked` 结果和 error-level 诊断，不得部分执行。
4. `B-004`：caller 提供的 role、risk、project/scope、branch、as-of、include-superseded 与 token
   budget 必须进入 plan 并约束执行。任何 keyword fallback、rerank 或 enrichment 都不得把结果扩展到
   caller 未授权的 project、branch、owner、suppressed 或 temporal scope。
5. `B-005`：新增的 MCP `context_bundle` tool 与 REST `POST /api/v1/context` 必须接受完整
   `ContextRequest` 及可选显式 intent，并返回 versioned `ContextBundle`/`ContextAudit`。显式 intent
   始终优先；不支持/拼写错误的 intent、role、risk 或非法 budget/scope 返回稳定的 4xx/MCP
   invalid-request 错误。现有 MCP/REST `search` 保持兼容；若后续复用 router，必须使用同一 typed
   request/plan executor，不能复制第二套映射。字段缺省时遵守当前 rollout mode，不得把猜测伪装成
   caller intent。
6. `B-006`：每个 channel 必须恰有一个 plan entry。disabled channel 不得读取数据或贡献分数；
   contribution cap 在 fusion 前后都必须可验证，timeout 按 plan 选择 `skip_channel` 或
   `fail_closed`。canonical FTS 基础证据失败时不得返回伪成功；可跳过 channel 的失败必须保留
   error/degradation reason，不能静默消失。
7. `B-007`：high-risk request 必须至少使用 `trusted` 证据、禁用 generated enrichment、要求 Top 1
   具有 canonical evidence，并在低证据时 abstain。空结果必须是带 reason 的合法空 bundle/search
   response；不得用 stale、superseded、quarantined、raw fallback 或跨 scope 数据填充结果。
8. `B-008`：`generated_enrichment` 必须与 `canonical_fts`、`canonical_vector` 分开归因，保留
   canonical source reference，并受独立 weight 与 max-contribution 限制。corrupted enrichment、
   缺失 projection、错误 source binding、重复生成文本或同一 projection 同时命中 FTS/vector 的
   fixture 不得获得两份独立 canonical 权重，不得单独把结果推到 high-risk Top 1；验证失败必须按
   plan 降级或阻断并留下 reason code。
9. `B-009`：Router 只决定 rerank 是否参与、candidate N、output k、canonical Top 1 要求与 timeout
   fallback；GH-851 的模型、eligibility 与执行机制保持唯一实现。rerank off、timeout、模型不可用或
   结果无效时，输出必须遵守 plan 的 fallback 且保留执行前 canonical order，不得重新扩大候选集。
10. `B-010`：retrieval 执行结果必须进入 versioned `ContextBundle` 与 `ContextAudit`。
    `ContextAudit` 至少记录 retrieval `policy_version`、resolved intent、`plan_hash`、每个 channel 的
    selected/disabled/degraded 状态、candidate/selected/dropped 数、reason codes、token estimate、
    latency 与 abstention。bundle、audit 与实际执行必须引用同一 plan hash；不匹配时 fail closed。
11. `B-011`：所有声明 router 行为的 memory benchmark 与 coding benchmark run artifact 必须携带
    resolved intent、retrieval policy version、plan hash、rollout mode、degradation/abstention 与
    implementation/dataset fingerprint。verifier 必须拒绝空 hash、未知 policy、hash 不一致、缺失
    fingerprint 或声称 `intent_router` 却没有 audit 的 artifact。
12. `B-012`：六个 intent 必须各有独立、确定性、离线 golden slice，覆盖 Recall@k、nDCG、
    abstention、stale-followed、irrelevant injection、p50/p95 latency、token estimate 与
    memory-hurt。每个 slice 同时包含 happy path、空/无证据、stale/superseded、scope mismatch、
    channel failure 与 unknown-intent 对照；fixture 不依赖 live LLM、网络或用户真实 memory。
13. `B-013`：默认启用决策必须来自同一 head、同一 dataset fingerprint 的
    `static_fusion` vs `intent_router` ablation。只有目标 intent slice 达到预先声明的显著改善、
    非目标 slice 无超阈值退化、memory-hurt 不增加、p95 latency 在预算内、unknown fallback 不差于
    static path 且 artifact verifier 通过，default gate 才可切换；任一指标缺失、样本不足或 stale
    report 都必须保持当前静态默认。
14. `B-014`：门禁通过前，未显式请求 router 的现有 CLI/MCP/REST caller 保持当前 static-fusion
    语义；显式合法 intent 可 opt in 到 router。默认切换必须有 versioned rollout mode、可见状态和
    无数据迁移的 rollback；rollback 只改变路由选择，不删除 audit、benchmark 或 canonical memory。
15. `B-015`：并发相同请求、分页、重试与 adapter 间调用必须复用相同 policy/plan semantics。
    同一 snapshot 的分页不得因 clock、随机 channel 顺序或重复 enrichment 改变 plan hash、排序或
    contribution；重试不得重复计入同一 canonical/projection pair。
16. `B-016`：foreground router 路径不得新增必需 LLM 或网络调用。channel timeout、DB/provider
    error、取消或部分失败必须按 plan 明确 degrade/abstain/block，并在 API/MCP/CLI debug/audit 中
    输出安全、无 memory 正文的诊断；不能用 warning + 空字段伪装成功。
17. `B-017`：PR #940 Phase A 的 DTO、planner、mapping tests、stable hash 与 `context-plan` 是本
    issue 的部分实现证据，不是剩余执行/评测验收。packet、current contract 与 PR body 必须持续标注
    partial，PR 使用 `Refs #934`；只有 `B-001`–`B-016` 的实现、测试、default gate 结论和完整
    verification 都有当前证据后才能关闭 GH-934。

## 验收标准

- [ ] 原 PR #940 的 Phase A 与本 packet/current contract 一致，六种 intent mapping、plan schema、
      high-risk 收紧、stable hash 与 `context-plan` 的 focused tests 通过。
- [ ] 真实 DB-backed retrieval 执行完全受 `RetrievalPlan` 控制，MCP/REST `context_bundle` 显式
      intent 路径返回
      同一 plan/audit，并覆盖 invalid/empty/error/degradation/abstention。
- [ ] corrupted enrichment 与 duplicate-signal fixtures 证明 generated signal 不冒充 canonical、
      不双重加权、不越过 cap 且不能成为 high-risk Top 1。
- [ ] `ContextBundle`/`ContextAudit` 与 memory/coding benchmark artifact 携带并验证相同
      `policy_version`/intent/plan hash/fingerprints。
- [ ] 六个 per-intent golden slice 与 unknown fallback slice 生成可复现报告，指标包含 Recall@k、
      nDCG、abstention、stale-followed、irrelevant injection、latency、token estimate、memory-hurt。
- [ ] `static_fusion` vs `intent_router` ablation 在同一 head/dataset 上完成；default gate 依据预声明
      阈值明确输出 enabled 或保持 static，不允许人工挑选单个改善样本代替完整矩阵。
- [ ] README、Architecture、current GH934 contract、SpecRail packet、API/MCP schema、benchmark
      schema、eval baseline/thresholds、CHANGELOG 与版本 surfaces 与最终行为同步。
- [ ] focused tests、artifact verifier、eval gates、`cargo fmt --check`、`cargo check`、`cargo test`、
      `cargo clippy --all-targets -- -D warnings`、JS tests、plugin version sync 与完整 PR preflight 通过。

## 边界情况

### Boundary checklist

| 边界类别 | 结论 |
| --- | --- |
| Empty / missing input | covered: `B-002`, `B-005`, `B-007`, `B-012` |
| Invalid enum / schema / budget | covered: `B-002`, `B-005`, `B-010`, `B-011` |
| Authorization / scope / suppression | covered: `B-004`, `B-007`, `B-008` |
| Error / timeout / provider failure | covered: `B-003`, `B-006`, `B-009`, `B-016` |
| Degradation / abstention | covered: `B-006`, `B-007`, `B-009`, `B-010`, `B-016` |
| Concurrency / retry / pagination | covered: `B-015` |
| Compatibility / rollout / rollback | covered: `B-013`, `B-014`, `B-017` |
| Evidence / audit integrity | covered: `B-008`, `B-010`, `B-011`, `B-013` |
| Corrupted or duplicate projection | covered: `B-008`, `B-015` |
| Cancellation / partial completion | covered: `B-006`, `B-016` |

- 显式 intent 与关键词冲突：显式 intent 获胜，audit 标记 `explicit_intent`。
- `SessionStart` 被显式传给 routable search：按现有 Phase A 契约拒绝或映射为
  `explore_history` 的保守 policy，并输出 `session_start_not_routable`；不得执行未声明的第七种
  router intent。
- multi-hop、explain 与 intent 参数组合不受支持时返回稳定错误；不能静默忽略 intent。
- raw archive fallback 只能遵守显式 plan/scope/trust/abstention；high risk 不得因 curated 结果不足
  自动放入 raw transcript。
- plan 编译成功但执行时 policy version 已变化：拒绝 hash/version 不匹配，caller 重新编译；不得
  用旧 plan 运行新 policy。
- default-gate report 与当前二进制、fixture 或 schema fingerprint 不一致：报告 stale，默认保持
  static。

## 发布说明

Phase A 保持现有 `remem context-plan` 实验命令。执行 wiring 首先以显式 intent opt-in 与审计字段
发布；未指定 intent 的既有 caller 在 default gate 通过前保持 static-fusion。default gate 若最终
通过，再以 versioned rollout mode 切换默认，并在 `remem status`/debug 输出中显示 active mode、
policy version 与 rollback 方法。该变更不迁移或删除 memory 数据。
