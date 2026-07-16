# Product Spec

Status: Draft — human `spec_approval` blocked on missing prerequisite evidence

## Linked Issue

GH-850（Refs #850；Epic #849）

complexity: large

## 用户问题

用户写入的记忆通常使用当时任务中的原始措辞，而后续查询可能使用同义词、概括语或另一种
表达方式。当前精确词法检索与基于原文的 embedding 因而可能同时漏掉语义相关记忆；已有
golden paraphrase slice 在当前 `origin/main` 基线上为零，无法证明改写查询的可召回性。

GH-850 提议为每条记忆生成只参与索引的简短上下文与同义关键词，同时要求原始记忆和上下文
注入内容不被改写。该能力会跨越写入、FTS、embedding、后台回填、doctor、评测和安全边界，
必须先固定失败、并发、隐私与回滚契约，不能把“生成成功”本身当作检索质量证据。

Issue #850 与 Epic #849 引用了
`docs/research/agent-memory-optimization-research-2026-07.md`，但该文件不在本草案基线
`origin/main@37f391ca704ae11ec811c330cdccdbc1527ccd49` 中。本草案不引用、转述或假定该未见
报告的内容。报告进入可审查的 immutable commit，或 maintainer 明确提供等价的可审查证据并
记录来源之前，本 spec 不得获得 human `spec_approval`，GH-850 也不得进入
`ready_to_implement`。

## 目标

- 为每条可检索记忆维护一个唯一、可重建、只用于检索的 enrichment 表面，使不同措辞的查询
  能被 FTS 和 embedding 两条通道共同利用。
- 保持 title/content 等 canonical memory bytes 以及注入/导出/API 中的记忆正文不变。
- 让存量回填有界、幂等、可中断且不阻塞 hook 或普通写入。
- 对生成失败、覆盖不足、并发失效、隐私泄漏和指令投毒提供可诊断、fail-visible 的行为。
- 用确定性的 paraphrase golden gate 证明收益，并保护现有检索与 abstention 指标不回退。

## 非目标

- 不新增第二套记忆表、第二套 FTS 或面向客户端的 `retrieval_text` 公共字段。
- 不改写、压缩或替换用户保存的 title/content、evidence、timestamps、scope 或 ownership。
- 不把 enrichment 文本直接注入 prompt、展示为记忆正文、导出到 pack/Markdown，或加入公共
  REST/MCP DTO。
- 不调整 hybrid fusion 权重、top-k、scope/branch/status 过滤或 poisoning acknowledgement
  规则。
- 不在 hook/写入响应路径执行 GH-850 的 LLM enrichment，也不让该路径等待 GH-850 回填。该
  约束不改变既有 embedding provider 的同步、联网或降级合同，也不声称配置为 OpenAI 的既有
  embedding 调用是离线的。
- 不借 GH-850 解决 Epic #849 的其他独立子题；特别是不放宽 GH-855 的 poisoning 防线。
- 不依据当前缺失的研究报告声明具体百分比收益、成本或推荐阈值。

## Behavior Invariants

1. `B-001`：spec approval 必须以可审查的研究依据为前置证据。若 issue 所引用报告缺失、为空、
   无 immutable revision 或与本 issue 无可追踪关联，即使草案、测试方案和代码上下文完整，状态
   仍为 blocked；不得用 issue 中的二手摘要冒充已审查报告。
2. `B-002`：每条记忆最多只有一个 authoritative index-only enrichment 表面。它可以由既有等价
   字段承载，但不得同时创建彼此可能漂移的 `search_context` 与 `retrieval_text` 两份真相。
3. `B-003`：生成、回填、重试和检索均不得修改 canonical title/content bytes。enrichment 不得
   出现在注入、API、MCP、pack、Markdown 或用户可见正文中；固定同一批 memory IDs 时，注入
   序列化的正文 bytes 必须与 enrichment 前完全一致。
4. `B-004`：成功的 enrichment 必须是闭集结构：一条有界上下文句和一个有界同义关键词列表。
   缺字段、空字符串/空列表、越界数量或长度、额外字段、非法 Unicode/control text、非 JSON、
   截断 JSON 或无法验证的输出均视为失败，不能发布为 ready index。
5. `B-005`：FTS 与启用的 embedding 通道必须消费同一版本、同一条记忆的 authoritative
   enrichment snapshot；任一通道不得使用另一版本或重新独立生成一份文本。显式关闭 embedding
   时只跳过向量通道，不影响 FTS 使用同一 snapshot。本条只禁止请求等待 GH-850 enrichment/
   backfill，不改变或重新描述既有 embedding provider（包括 OpenAI）的网络与同步行为。
6. `B-006`：新写入或 canonical 内容更新必须先同步提供不依赖 LLM 的原文索引，随后才异步
   enrichment。hook 和普通写入不得等待 GH-850 生成、GH-850 worker lease 或 GH-850 存量回填；
   enrichment 未 ready 时 title/content 仍可由 FTS 检索，已启用 embedding 时仍保留基于当前可用
   索引文本的向量。既有 embedding 调用是否联网/同步继续由其现行合同决定。
7. `B-007`：存量回填必须按有界批次推进，覆盖 retrieval 可见的 active、stale、archived 行，
   可安全重复和中断。每次 AI 调用前必须持久化绑定 source identity、generator/security-policy
   version、attempt identity、lease owner 与 expiry 的 claim；未过期 claim 或已经 ready 的相同身份
   不得再次调用 AI。失败行可在 lease 到期后用新 attempt 重试，但一次失败不得因并发重复 claim
   造成重复调用/收费、busy loop 或阻塞其他行；provider 不支持 idempotency 时，不虚假承诺进程在
   provider 接收后崩溃这一不可判定窗口的计费 exactly-once。
8. `B-008`：AI、解析、校验、redaction、poison scan、embedding 或数据库写入任一步失败时，
   系统必须保留/恢复该行原文索引可见性；source identity 未变化时保留最后一个一致 snapshot，
   source 已变化时只允许新原文/确定性 fallback，并写 error-level diagnostic。失败状态只能由当前
   claim 的 source、generator/security-policy version、attempt/lease CAS 写入；迟到失败不得覆盖已
   ready 的结果或较新 attempt。失败不得被记作覆盖成功，不得静默清空 FTS/向量，也不得返回伪造
   enrichment。
9. `B-009`：任何会改变 enrichment 输入的 canonical 更新必须立即使旧版本失效；在新版本 ready
   前，检索只可使用更新后的原文/确定性 fallback，不得继续使用与旧内容绑定的 enrichment；仅
   修改 canonical 字段而未显式修改 enrichment metadata 的写入，也必须保证旧 enrichment-only term
   不再由 FTS MATCH 或 vector channel 命中。
10. `B-010`：并发生成的成功与失败都必须以 claim 时的 source identity、generator/security-policy
    version、attempt identity 和 lease owner 为条件提交。若生成期间记忆被更新、删除、改为不可
    处理状态，lease 被新 attempt 接管，或另一 worker 已提交 ready，旧 worker 必须 no-op/rollback，
    不能覆盖新内容、复活删除行、把版本倒退或用迟到失败清除 ready。
11. `B-011`：enrichment、FTS snapshot 和启用通道的向量更新必须表现为单行一致提交；取消、
    timeout、进程退出或 crash 发生在提交前不得留下半 ready 状态，发生在提交后重试必须幂等。
    embedding 显式为 off 是唯一允许“无向量但 ready for FTS”的状态，且必须可诊断。
12. `B-012`：迁移后的历史行、NULL/空 enrichment 行和旧客户端继续可用。初始 v070 迁移不得同步
    调用 AI 或等待 AI backfill；历史行在回填完成前继续通过 canonical title/content 检索，旧客户
    端看不到新字段且无需升级 payload schema。未来 security-policy version bump 属于 fail-closed
    例外：在 deterministic fallback 全量恢复并重新建立安全索引前不得对外提供检索，失败必须阻断
    启动而不能继续使用旧 policy enrichment。
13. `B-013`：`remem doctor` 必须报告 eligible、ready、pending/failed-to-cover 数量、当前
    generator/security-policy versions 和覆盖率；零 eligible 可明确为 OK，低覆盖必须 Warn/Fail
    并给出恢复动作，数据库/计算错误必须 Fail 而不能伪装成 0/0 或 100%。
14. `B-014`：enrichment 只继承用户已配置的 memory AI executor/provider 权限，不新增网络、
    repository、跨项目或跨用户权限。每次生成只能读取一条记忆的允许字段，输入输出均须先进行
    secret redaction 和字节上限控制；不得把其他记忆、绝对项目路径、凭据或未授权 evidence 拼入
    prompt/索引/日志。
15. `B-015`：canonical memory 与生成输出都按不可信数据处理。生成 prompt 必须把原文置于数据
    边界，输出不得包含执行指令；若 enrichment 命中当前 instruction-pattern/opaque-payload 防线，
    整份新 enrichment 必须拒绝、error 记录并回退，不能因 canonical source 曾被人工
    acknowledge 就自动信任新生成文本。security-policy version 一旦提升，所有旧-policy enrichment
    必须先失效并 fail-closed 回 deterministic fallback，不能等新 AI 生成后才停止使用旧文本。
16. `B-016`：diagnostic、doctor 与 eval 证据必须绑定 generator version、security-policy version、
    source/index/attempt identity 和实际执行的通道；缺失或旧版本证据不能证明 ready。日志不得包含
    原文、生成全文或 secret，但必须包含可定位的 memory id、阶段和非敏感错误类别。
17. `B-017`：发布门禁必须在隔离 fixture 上确定性复现，CI 不得依赖实时 LLM/网络。质量 gate 的
    enrichment 必须来自被冻结的 production generator artifact，记录 generator/security-policy
    version、prompt hash、executor、exact model/revision、corpus 与输出 hash；人工编写 context 只能
    验证 FTS/vector wiring，不得作为质量提升证据。与 exact `origin/main` 基线相比，paraphrase
    slice 的 `hit_at_k`、`evidence_recall_at_k`、`mrr_at_10` 必须都从 0 严格提升，且既有非
    paraphrase slice、abstention 和 scope-leak 门禁不得超过现有允许回退。
18. `B-018`：回滚不得删除或改写 canonical memory。停止新 enrichment 后，系统必须能通过
    forward-compatible、可中断的重建恢复确定性 fallback 与匹配向量；回滚期间任何未完成行仍
    保持原文 FTS 可见，不能通过 down migration 丢弃审计或让旧 enrichment 冒充当前版本。

## 验收标准

- [ ] 研究报告或 maintainer 认可的等价证据已绑定 immutable revision，并在 human review 后
      解除 `B-001` blocker；解除前没有 `spec_approval` / `ready_to_implement`。
- [ ] schema 与 retrieval 只有一个 index-only enrichment truth，canonical title/content bytes
      以及所有注入/输出 DTO 保持不变。
- [ ] focused FTS 与 vector 测试证明两通道消费同一 snapshot；embedding off 行为明确可诊断。
- [ ] 新写入、存量回填、更新失效、持久化 claim/lease/attempt、重复执行、双 worker race、lease
      takeover、迟到成功/失败、取消/crash 和失败 fallback 均有确定性测试。
- [ ] 生成/embedding/事务失败会产生 redacted error-level diagnostic，原文 FTS 仍可命中且
      doctor 不虚报覆盖。
- [ ] doctor 覆盖率与 generator/security-policy versions 可见，数据库错误 fail closed。
- [ ] poisoning、secret redaction、跨项目隔离、输出闭集与长度上限均有正负例。
- [ ] deterministic golden replay 使用冻结的 production generator/model artifact，paraphrase 三项
      指标严格高于 exact-main 的零基线，其他现有 gates 不回退；人工 context 只证明 wiring，CI
      不发起 live AI call。
- [ ] migration、回填与 rollback rehearsal 证明可中断、幂等、不会阻塞 hook。
- [ ] spec-only PR 使用 `Refs #850` / `Refs #849`，不关闭 implementation issue；最终 review、
      merge、release 仍保留 human gates。

## 边界情况

| Boundary category | Verdict |
| --- | --- |
| Empty / missing input | covered: `B-004`, `B-008`, `B-012` |
| Error and failure paths | covered: `B-008`, `B-011`, `B-013`, `B-015` |
| Authorization / permission | covered: `B-001`, `B-014`, `B-015` |
| Concurrency / race / ordering | covered: `B-006`, `B-009`, `B-010`, `B-011` |
| Retry / repetition / idempotency | covered: `B-007`, `B-010`, `B-011`, `B-018` |
| Illegal state transitions | covered: `B-001`, `B-009`, `B-010`, `B-016` |
| Compatibility / migration | covered: `B-002`, `B-003`, `B-012`, `B-018` |
| Degradation / fallback | covered: `B-006`, `B-008`, `B-011`, `B-013` |
| Evidence and audit integrity | covered: `B-001`, `B-013`, `B-016`, `B-017` |
| Cancellation / interruption / partial completion | covered: `B-007`, `B-011`, `B-018` |

组合边界还包括：authorized memory AI 配置存在但研究证据缺失时仍不可批准 spec（`B-001` +
`B-014`）；lease takeover 后的迟到失败不得复用旧 attempt 或覆盖 ready（`B-007` + `B-008` +
`B-010`）；embedding off 时允许 FTS ready，但不得把向量覆盖伪装为成功（`B-011` + `B-013`）；
canonical poison 被人工 acknowledge 也不能自动批准新生成 poison，security-policy bump 也不得
继续使用旧 enrichment（`B-015`）。

## 发布说明

该能力需要 additive schema migration 和后台渐进回填。升级后无需改客户端 payload；回填期间
搜索继续使用原文 fallback，doctor 显示实际覆盖。用户可见发布说明必须明确：enrichment 仅
影响候选召回，不会改写或注入生成文本；启用的 memory AI executor 可能处理经过 redaction、
限长的单条记忆数据；失败会保留原始检索并显式诊断。

本文件是 draft，不构成 `spec_approval`。研究前置证据、human spec review、
`ready_to_implement`、最终 PR review、merge、security decision 与 release authorization 均
保持为独立 human gates。
