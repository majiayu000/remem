# Context Bundle v1 完整合同 — Product Spec

## Linked Issue

GH-932

## 用户问题

remem 已经能在 SessionStart、CLI、MCP 和 REST 中提供多类记忆，但调用方仍需自行理解搜索、
过滤、预算、时间有效性和降级细节。随着 enrichment、rerank、graph projection、host-native
memory 与更多 owner/scope 出现，同一任务可能在不同入口得到不同选择、不同截断或无法解释的
空结果。

PR #938 已合并第一阶段基础设施：versioned `ContextRequest` / `ContextPlan` /
`ContextBundle` / `ContextAudit`、deterministic `plan` / `execute`、稳定 `plan_hash`、
caller-provided candidates 上的基础 scope/budget/audit，以及
`full` / `canonical_only` / `blocked`。它是 #932 的部分实现，不是 issue 完成证据：
生产 DB executor、完整 Bundle 语义、role/as-of/worktree 执行、严格 rendered budget、
SessionStart 切换、MCP/REST、doctor 与 benchmark hash 仍未落地。

用户需要一个单一、可审计的 context compiler：输入任务与明确 scope，先得到确定性计划，再从
同一数据库快照得到有界、可解释、可重放的 Context Bundle；任何入口都不能把 generated 或
graph-derived projection 冒充 canonical truth，也不能静默丢失数据。

## 目标

- 完成一个 versioned、deterministic、DB-backed 的 `plan(request)` /
  `execute(plan)` Context Bundle 合同。
- 让 project、branch、worktree、role、`as_of_epoch`、risk 和 supersession policy 在
  planner 与 executor 两层都真实生效，而不只是出现在 DTO 中。
- 输出完整的 current truth、decisions、constraints、failure lessons、workstreams、
  conflicts、abstentions、evidence refs、freshness 与 audit。
- 对最终可注入 rendered context 执行严格总预算和 section budget，并审计全部截断。
- 让 SessionStart、实验性 MCP/REST、doctor 和 coding-bench 使用同一 compiler 事实源。
- 保留旧 SessionStart 路径作为显式、可测试的 rollback，直到兼容性门通过。

## 非目标

- 不重写所有 retrieval channel，不在本 issue 中引入新的 graph/rerank 算法。
- 不引入 foreground LLM、远程模型调用、自动下载或运行时网络依赖。
- 不把 Context Bundle 变成 agent runtime、任务队列、lease 或 harness coordination store。
- 不把 generated enrichment、embedding 命中或 graph expansion 提升为 canonical memory。
- 不承诺实验性 Context Bundle DTO 永久不变；不兼容变更必须提升 schema/policy version。
- 不改变现有 memory capture、extraction、promotion 或长期持久化语义。
- 不把 audit/debug payload 直接注入模型正文，也不在 doctor 中暴露 memory payload。

## Behavior Invariants

### 合同与确定性

1. **B-001 — Versioned closed contract。** `ContextRequest`、`ContextPlan`、
   `ContextBundle`、`ContextAudit` 及其公共 enum 都携带或受同一显式 schema version
   约束；未知 version/enum 必须显式拒绝，不能 alias、猜测或静默降级。
2. **B-002 — Deterministic plan。** 相同的 normalized request 与 compiled policy
   必须产生 byte-identical plan JSON 和相同 `plan_hash`；时钟、随机数、环境遍历顺序和
   DB 行顺序不得进入 plan。
3. **B-003 — Deterministic execution。** 相同 DB snapshot、plan 和 policy 必须产生
   相同 Bundle、rendered context、`audit_hash` 与 machine-readable reason；排序必须有
   完整稳定 tie-breaker。
4. **B-004 — Plan/execute separation。** 调用方可只生成 plan 而不读取 memory payload；
   execute 必须验证收到的 exact plan（含 hash、schema、policy、scope），不能在执行时静默
   重写计划。

### 完整 Bundle 与 provenance

5. **B-005 — Complete semantic sections。** Bundle 必须稳定输出
   `current_truth`、`decisions`、`constraints`、`failure_lessons`、`workstreams`、
   `conflicts`、`abstentions`、`evidence_refs`、`freshness` 和 `audit`；无数据时对应数组
   为空、summary 明确为 empty，禁止填 placeholder 或虚构内容。
6. **B-006 — Canonical/derived separation。** 每个 item 都必须带 `source_kind`、
   `canonical_ref`、可选 `projection_ref`、`evidence_refs`、validity 与 trust；
   generated/graph-derived item 没有 canonical back-reference 时必须 abstain/drop，
   不能成为 current truth。
7. **B-007 — Complete audit。** 每个 DB candidate 最终恰好对应 selected、dropped、
   abstained 或 conflict 中的一种终态，并记录 channel、scope filters、validity、trust、
   预算计数、selection/drop reason、policy/schema version、plan hash 与 degraded mode。
   重复、遗漏或同一 candidate 多个互斥终态必须失败。
8. **B-008 — Conflict and freshness visibility。** 多个互相矛盾且不能按 current-state/
   temporal policy 唯一裁决的 canonical facts 必须进入 `conflicts` 或 `abstentions`，
   不能任选其一；freshness 必须区分 current、stale、superseded、unknown 和 as-of
   reference time。

### Scope、时间与角色

9. **B-009 — Project/worktree safety。** planner 验证 project 非空并规范化 worktree；
   executor 在读取与发布前再次验证 worktree 属于同一 project/repository。mismatch 或无法
   安全确认时为 `blocked`，不能退回全局/当前目录。
10. **B-010 — Branch safety。** branch filter 在 DB candidate query 和 executor
    两层执行；branch-specific item 不得跨 branch 泄漏。允许的 branchless/global
    compatibility 行为必须有固定 reason 和测试，不能由空字符串隐式决定。
11. **B-011 — Role policy is effective。** `coder`、`reviewer`、`planner`、
    `researcher` 是封闭角色集，并进入 plan hash。每个角色使用固定、可测试的 section
    priority/eligibility policy；role 只能收紧或重新分配相关内容，不能放宽 project、
    trust、suppression 或 temporal safety。
12. **B-012 — As-of truth。** `as_of_epoch` 在 planner、DB query 与 executor
    三处一致生效：未来创建、当时尚未生效、当时已过期/invalidated 的 item 不可 selected；
    历史查询不能使用当前 wall clock 代替 request time。缺少足够 temporal provenance 时
    必须标记 unknown/abstain。
13. **B-013 — Supersession policy。** `include_superseded=false` 默认只返回 as-of
    当前有效 truth；显式 true 才可返回历史版本，且历史 item 仍带
    `superseded` validity 和 reason，不能混入 current truth。
14. **B-014 — Risk cannot weaken safety。** risk class 进入 plan hash 并使用封闭 policy；
    high risk 可以收紧 evidence/trust/abstention 要求，但任何 risk 值都不能扩大权限、
    绕过 quarantine/suppression 或允许无 provenance projection。

### 预算、降级与失败

15. **B-015 — Strict rendered budget。** 总 `token_budget` 与各 section budget 作用于
    最终 rendered context 的全部可注入字符（含标题、分隔、引用和固定前缀），使用 versioned、
    deterministic estimator 计数；最终 audit 中的 rendered estimate 必须不超过总预算，
    每个 section 也不得超限。只计算 `ContextItem.text` 不算满足。
16. **B-016 — Audited truncation。** 超预算时只按固定优先级和稳定 item 边界删除/截断，
    每个被影响 item 都记录 `section_budget`、`total_budget` 或固定细分 reason；
    禁止产生半个 UTF-8 字符、截断 canonical/evidence identity 或让 audit 与正文不一致。
17. **B-017 — Bounded degradation。** enrichment/vector/rerank 不可用时可以
    `canonical_only`，并删除所有 derived item；canonical schema、DB integrity、scope safety
    或 plan validation 无法保证时只能 `blocked`。任何 degraded/blocked 原因都必须进入
    audit、error-level diagnostic 和对应 transport 状态，不能 warning + fallback。
18. **B-018 — Empty/error/cancellation atomicity。** 合法但无候选时返回成功的 empty
    Bundle；DB/query/render error、deadline 或 cancellation 不得返回部分 Bundle。执行要么
    发布一个已验证的完整结果，要么返回明确 error/blocked outcome。
19. **B-019 — Snapshot concurrency。** 一次 execute 的所有 channel 从同一只读数据库
    snapshot 读取；并发 capture/promotion 不得造成一半旧、一半新结果。并发相同请求不得写
    runtime DB，也不得互相污染 plan/audit hash。
20. **B-020 — Offline and permission behavior。** plan、execute、render、doctor 与
    benchmark hash 路径不需要网络。REST 继续使用现有 localhost bearer auth，MCP 继续使用
    现有本地进程边界；缺 token/未授权请求必须在读取 memory payload 前拒绝。

### 入口、兼容与可观测性

21. **B-021 — SessionStart single source。** 启用新路径时，SessionStart 从 DB-backed
    Bundle renderer 产生上下文，不能再独立执行另一套 selection/budget。兼容 fixture 在同一
    snapshot 下证明旧路径可见语义等价；切换失败不得双重注入或静默回退。
22. **B-022 — Explicit rollback。** 新 SessionStart 路径先以显式 feature/config gate
    上线；rollback 只切回已测试旧 renderer，不改变 DB/schema，不删除 audit evidence。
    gate 状态与实际路径必须在 diagnostics/doctor 可见。
23. **B-023 — MCP/REST parity。** 提供 versioned experimental plan 与 bundle surface；
    MCP 与 REST 接受同一语义 request、返回同一 schema/hash/reason。REST 使用 POST body，
    不能把 task 或 payload 放入 URL；invalid、blocked、unauthorized 与 internal error 有稳定
    区分，且 schema/backward-compatibility tests 固定。
24. **B-024 — Doctor without payload。** `remem doctor` 文本和 JSON 报告 compiler
    schema/policy、DB executor readiness、SessionStart selected path、MCP/REST capability
    与 degraded/blocked reason；只输出 plan summary/hash 和状态，不输出 memory text、
    query、evidence 内容或 secret。
25. **B-025 — Benchmark provenance。** 每个 remem-backed coding-bench run artifact
    保存 context schema version、policy version、`plan_hash`、`audit_hash`、degraded mode
    与 rendered budget evidence；缺失、hash 不匹配、非同 head/fixture 的证据使该 run 的
    context contract 失败。
26. **B-026 — No foreground LLM/network。** 所有入口使用 deterministic local logic
    和现有本地 DB；不得为 plan/execute/render 自动调用 LLM、下载模型或访问网络。
27. **B-027 — Compatibility and accessibility。** 未启用新 SessionStart gate 时现有
    CLI/MCP/REST 行为保持不变；实验性 surface 显式标 version。所有状态与 reason 都有
    machine-readable 字段，文本输出不能只用颜色、图标或视觉顺序表达关键信息。

## 验收标准

- [ ] `B-001` 至 `B-027` 全部在 `tech.md` 与 `tasks.md` 中有实现区域和确定性验证映射。
- [ ] DB-backed executor 从同一 read transaction 读取真实 remem 数据，不接受公共调用方伪造
      candidate 列表作为生产路径。
- [ ] 完整 Bundle sections、provenance、conflict/abstention、freshness 和 audit schema snapshot
      通过。
- [ ] 相同 snapshot/request/policy 的 plan、Bundle、rendered context、`plan_hash` 和
      `audit_hash` byte-stable。
- [ ] project/branch/worktree/role/as-of/risk/supersession 的正反矩阵通过，未知值 fail closed。
- [ ] 最终 rendered context 与每个 section 的 versioned estimate 严格不超预算，所有截断理由
      可从 audit 复现。
- [ ] `full`、`canonical_only`、`blocked`、empty、DB error、cancellation 和并发 snapshot
      fixtures 全部通过，且没有 partial output。
- [ ] SessionStart compatibility、single-path injection、gate 与 rollback tests 通过。
- [ ] MCP/REST schema/parity/auth/error tests 通过；doctor 文本/JSON 不泄露 payload。
- [ ] coding-bench artifact 保存并校验 schema/policy/plan/audit hash 与预算 evidence。
- [ ] deny-network fixture 证明 plan/execute/SessionStart/doctor/benchmark 路径无 foreground
      LLM 或网络调用。

## 边界情况清单

| Boundary | Required behavior |
| --- | --- |
| Happy path | 一个验证通过的 plan 在同一 DB snapshot 产生完整、有界、可审计 Bundle |
| Empty DB / no eligible data | 成功返回 empty arrays + freshness/audit summary；不生成占位记忆 |
| Invalid schema/policy/hash | 在读 payload 或发布结果前显式拒绝；`blocked`/transport error 可区分 |
| DB open/query/schema error | error-level diagnostic；无 partial Bundle，无 silent empty fallback |
| Loading/intermediate state | 同步 surface 不发布中间态；只有完整终态或明确 error |
| Cancellation/deadline | 丢弃临时结果；无部分正文、无半份 audit、无 DB 写 |
| Permission/auth | REST 未授权在 DB payload read 前拒绝；MCP 不扩大现有本地权限边界 |
| Offline/network failure | 正常工作且零网络；若现有 optional enrichment 不可用则按 policy 明确降级 |
| Concurrency/race | 同一 execute 固定 snapshot；并发写只影响下一次执行 |
| Project/worktree mismatch | `blocked`，不得退回 cwd/global |
| Branchless/global item | 仅按固定 compatibility/owner policy 处理，并记录 reason |
| Role/risk unknown value | schema reject；不能 alias 到默认值 |
| Historical as-of | 使用 request epoch；未来/过期/invalidated 数据不可泄漏 |
| Superseded conflict | 按显式历史 policy 返回或进入 conflict/abstention，不任意择一 |
| Derived source unavailable | `canonical_only`，所有 derived item 有 drop reason |
| Strict budget too small | 返回可验证的最小 empty/metadata outcome；不得超预算或切半 identity |
| MCP/REST compatibility | 相同 request/snapshot 得到相同 schema/hash/reason；transport wrapper 除外 |
| SessionStart rollback | 单次只走一个 renderer；rollback 不改 DB、不重复注入 |
| Accessibility | JSON/text 有稳定字段/标签；颜色和图标不是唯一状态载体 |

## 发布说明

这是对已合并部分基础设施的完整实现，不把 PR #938 重写为“未完成”。实现必须更新
`docs/specs/GH932/` 当前合同与 spec index，并在 changelog 中明确：

- internal schema/policy version 的提升；
- experimental MCP/REST surface；
- SessionStart gate 默认值与 rollback；
- doctor/benchmark 新 evidence 字段。

在人工批准 product/tech、issue 进入 `ready_to_implement`、implementation route gate
通过、CI/独立 review/merge gate 完成之前，本 spec 不授权生产代码、默认切换或 release。
