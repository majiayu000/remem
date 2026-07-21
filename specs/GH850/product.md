# Product Spec

Status: Draft — human `spec_approval` blocked on prerequisite evidence and security review

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
`origin/main@2dc41cb332ead83ff39f234444fc76fc50713f43` 中。本草案不引用、转述或假定该未见
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
2. `B-002`：每条记忆最多只有一个 authoritative index-only enrichment 表面；不得向用户暴露
   两份可能漂移的检索文本或要求客户端在两者间选择。
3. `B-003`：生成、回填、重试和检索均不得修改 canonical title/content bytes。enrichment 不得
   出现在注入、API、MCP、pack、Markdown 或用户可见正文中；固定同一批 memory IDs 时，渲染
   正文必须 byte-for-byte 不变。
4. `B-004`：成功的 enrichment 只包含一条有界上下文句和一个有界同义关键词列表。缺失、空值、
   越界、额外内容、非法文本、截断或无法验证的结果均视为失败，不得作为 ready enrichment。
5. `B-005`：FTS 与启用的 embedding 通道必须消费同一版本、同一条记忆的 authoritative
   enrichment snapshot；任一通道不得独立生成或使用另一版本。显式关闭 embedding 时只跳过
   向量覆盖，不影响 FTS；既有 embedding provider 的联网与同步合同不因本能力改变。
6. `B-006`：新写入或 canonical 内容更新后，原文检索必须立即可用，enrichment 随后异步完成。
   hook 和普通写入不得等待 enrichment 生成或存量回填；enrichment 未 ready 时不得造成召回空窗。
7. `B-007`：存量回填覆盖所有 retrieval 可见状态，按有界批次推进，并可安全重复、中断和恢复。
   同一版本的有效处理中任务不得因并发 worker 重复调用或 busy loop；provider 无法判定的崩溃
   计费窗口必须如实披露，不能宣称 exactly-once。
8. `B-008`：生成、解析、安全检查、embedding 或持久化任一步失败时，系统必须保留当前内容的
   原文检索，并记录 error-level diagnostic。失败不得被记作覆盖成功，不得静默清空任一检索
   通道，也不得用迟到失败覆盖较新的成功结果。
9. `B-009`：任何影响 enrichment 输入的 canonical 更新必须立即使旧 enrichment 失效；新版本
   ready 前只可使用更新后的原文或确定性 fallback。之后的无关更新也不得让旧 enrichment-only
   term 在 FTS 或 vector 中重新命中。
10. `B-010`：若生成期间记忆已更新、删除、变为不可处理状态，任务所有权已转移，或较新结果已
    ready，迟到的成功或失败都必须无副作用；不得覆盖新内容、复活删除行或倒退版本。
11. `B-011`：一条记忆的 enrichment 与启用检索通道必须一致发布。取消、timeout 或 crash 不得留下
    半 ready 状态，重试必须幂等；安全策略收敛失败时 retrieval 保持 fail-closed。embedding 显式
    关闭是唯一允许“FTS ready 但无向量”的状态，且必须可诊断。
12. `B-012`：升级后的历史记忆、空 enrichment 与旧客户端继续可用。schema 升级不得同步调用 AI
    或等待全量回填，旧客户端无需改变 payload。若 binary 与数据库要求的安全策略不兼容，普通
    retrieval/worker 必须 fail-closed；安全策略升级完成全部必要检索覆盖前不得重新开放。
13. `B-013`：`remem doctor` 必须报告 eligible、ready、pending/failed-to-cover 数量、当前
    generator/security-policy versions 和覆盖率；零 eligible 可明确为 OK，低覆盖必须 Warn/Fail
    并给出恢复动作，数据库/计算错误必须 Fail 而不能伪装成 0/0 或 100%。
14. `B-014`：enrichment 只继承用户已配置的 memory AI 权限，不新增网络、repository、跨项目或
    跨用户权限。每次生成只可读取一条记忆的必要字段，输入输出必须脱敏且有界；其他记忆、
    绝对项目路径、凭据或未授权 evidence 不得进入生成请求、索引或日志。
15. `B-015`：canonical memory 与生成输出都按不可信数据处理。命中当前指令投毒或 opaque payload
    防线的生成结果必须整份拒绝、error 记录并回退；对原文的人工 acknowledgement 不自动信任
    新生成文本。安全策略提升后旧 enrichment 必须先失效，恢复检索前须展示可能的网络、计费和
    模型下载影响并取得独立 human security approval。
16. `B-016`：diagnostic、doctor 与 eval 证据必须绑定 generator/security-policy version、源内容、
    索引结果、处理尝试和实际通道；缺失或过期证据不能证明 ready。日志不得包含原文、生成全文
    或 secret，但必须包含可定位的 memory id、阶段和非敏感错误类别。
17. `B-017`：发布门禁必须在隔离 fixture 上确定性复现，CI 不得依赖实时 LLM/网络。质量证据必须
    来自冻结的 production generator artifact，并绑定 generator、安全策略、prompt、executor、
    exact model/revision、corpus 与输出；人工 context 只能证明通道 wiring。相对 exact
    `origin/main`，paraphrase slice 的 `hit_at_k`、`evidence_recall_at_k`、`mrr_at_10` 必须均严格
    提升，既有非 paraphrase、abstention 与 scope-leak 门禁不得超过允许回退。
18. `B-018`：回滚不得删除或改写 canonical memory。停止新 enrichment 后，系统必须能通过
    forward-compatible、可中断的重建恢复确定性 fallback 与匹配向量；回滚期间任何未完成行仍
    保持原文 FTS 可见，不能通过 down migration 丢弃审计或让旧 enrichment 冒充当前版本。

## 验收标准

- [ ] 研究报告或 maintainer 认可的等价证据已绑定 immutable revision，并在 human review 后
      解除 `B-001` blocker；独立 security review 已批准 enrichment 的生成、投毒和外部调用边界；
      两者完成前没有 `spec_approval` / `ready_to_implement`。
- [ ] schema 与 retrieval 只有一个 index-only enrichment truth，canonical title/content bytes
      以及所有注入/输出 DTO 保持不变。
- [ ] focused FTS 与 vector 测试证明两通道消费同一 snapshot；embedding off 行为明确可诊断。
- [ ] 新写入、存量回填、更新失效、持久化 claim/lease/attempt、重复执行、双 worker race、lease
      takeover、迟到成功/失败、取消/crash 和失败 fallback 均有确定性测试。
- [ ] raw canonical update 在同一事务持久化 fallback/invalid identity；随后 unrelated access-count
      update 后旧 term 仍不命中、新 term 命中，FTS integrity clean。
- [ ] 生成/embedding/事务失败会产生 redacted error-level diagnostic，原文 FTS 仍可命中且
      doctor 不虚报覆盖。
- [ ] doctor 覆盖率与 generator/security-policy versions 可见，数据库错误 fail closed。
- [ ] poisoning、secret redaction、跨项目隔离、输出闭集与长度上限均有正负例。
- [ ] deterministic golden replay 使用冻结的 production generator/model artifact，paraphrase 三项
      指标严格高于 exact-main 的零基线，其他现有 gates 不回退；人工 context 只证明 wiring，CI
      不发起 live AI call。
- [ ] migration、回填与 rollback rehearsal 证明可中断、幂等、不会阻塞 hook。
- [ ] policy upgrade/downgrade 测试证明 DB floor 单调、旧 binary 在 retrieval/worker 前 fail closed；
      provider enabled 的 fallback vector 失败阻断 reopen，provider off 才允许无 vector。
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

本文件是 draft，不构成 `spec_approval`。研究前置证据、human security review、
human spec review、`ready_to_implement`、最终 PR review、merge 与 release authorization 均
保持为独立 human gates。
