# Product Spec

## Linked Issue

GH-855

complexity: large

## 用户问题

remem 已在 GH-672 建立候选 memory 的版本化 instruction-pattern 扫描、source trust、
quarantine、显式 acknowledgement，以及 memory/lesson 注入前复扫；但当前 capture/extraction
链路仍有两个可验证缺口：

- 捕获事件中的指令性文本只有在生成后的 candidate 文本仍命中模式时才会触发 quarantine。
  如果提取模型省略、转述或拆分了原始投毒文本，原始来源的风险不会随 evidence 传播，可能被
  “洗白”为普通候选。
- session rollup 会把模型输出直接写入 `session_summaries`，而 SessionStart 的 recent sessions
  注入没有应用现有 poisoning scanner 或 acknowledgement。由工具输出或 transcript 进入 rollup
  的指令性文本因此可能绕过 candidate 防线。

这会让攻击者把“忽略此前指令”“伪造最高权限”“静默执行命令”或不透明编码载荷藏入 hook
捕获内容，经 observation/candidate 或 session summary 路径成为后续 agent 可见上下文。安全修复
必须保留可审计的、已脱敏的原始捕获证据，同时阻止未确认的衍生内容成为 active memory 或注入
上下文；不能靠删除事件、静默跳过或仅加强 prompt 文案来掩盖风险。

Issue 引用的仓库研究报告
`docs/research/agent-memory-optimization-research-2026-07.md` 在本 spec 起草基线
`origin/main@37f391ca704ae11ec811c330cdccdbc1527ccd49` 中不存在。因此本文件不采纳该报告的
百分比或结论；报告的不可变 revision，或 maintainer 明确认可的等价一手证据，是
`spec_approval` 的前置条件。

## 目标

- 对 hook capture、observation extraction、memory candidate、session rollup 与 SessionStart
  注入建立同一套版本化、确定性的 poisoning 判定契约。
- 让原始 captured evidence 的命中结果随衍生 artifact 传播，阻止模型转述或遗漏造成的风险洗白。
- 在持久化、扫描和日志输出前保持 secret redaction；quarantine 不泄露原文，也不删除规范捕获
  证据。
- 将 session summary 接入现有 quarantine、governance acknowledgement、doctor/status 与审计
  机制，而不是创建第二套审核语义。
- 用确定性 adversarial eval 覆盖 tool output、transcript、instruction override、authority claim、
  opaque payload、双语文本、secret 混合载荷和无害引用。

## 非目标

- 不声称能识别所有语义级 prompt injection、加密/隐写载荷或未知未来模式；版本化模式集仍是
  可解释的保守防线。
- 不删除或改写规范的 captured event/raw archive 来“清理”投毒；它们继续作为已脱敏审计证据。
- 不替换 LLM extraction、session rollup 或自动 capture，也不以降低记忆质量来规避风险。
- 不把本工作扩展为通用 DLP/secret scanner；这里只要求现有 redaction 在新增边界无回归，并对
  LLM 生成文本重新执行同一 redaction。
- 不实现 GH-852 的 Claude/Codex 原生记忆导入。GH-852 后续新增来源必须复用本契约，但 GH-855
  不等待或假设该能力已存在。
- 不自动批准 quarantine，不从“来源可信”推导 acknowledgement，也不移除 human review、merge、
  release 或 security gates。

## Behavior Invariants

1. `B-001`：进入 poisoning 判定的文本必须先经过现有 secret redaction；捕获事件、transcript、
   observation 输出、candidate 输出、rollup summary/structured field/topic segment 与任何诊断
   preview 都不得把已识别 secret 的明文写入数据库、日志、status、doctor 或 eval artifact。
2. `B-002`：规范的 captured event 与 raw archive 即使命中 instruction pattern 也必须保留其已
   脱敏审计记录，不得删除、截断成空白或标记为“处理成功但未写入”；风险通过安全元数据传播到
   衍生 artifact。
3. `B-003`：同一版本化、确定性的 pattern set 必须覆盖现有 override、reader execution、
   concealment、authority claim 与 opaque payload 类别，并对英文、中文及规范化后的等价输入给出
   可复现的 `pattern_id@version`；未知、空白或越界元数据不能被解释为安全。
4. `B-004`：candidate 的最终 verdict 必须同时扫描其生成文本和全部被引用的已脱敏 source
   evidence。任一来源或产物命中时，即使模型省略、转述、拆分了命中文本，该 candidate 仍须进入
   现有 `quarantined` 状态且不得 auto-promote 或成为 active memory。
5. `B-005`：source evidence 缺失、事件 ID 越界、无法读取、redaction/scan 失败或 verdict 无法
   完整持久化时，extraction 不得默认使用 `safe`、空列表或较高 trust 继续；该次派生写入须原子
   失败并产生可定位的 error 级证据，规范原始 capture 保持可重试。
6. `B-006`：rollup 的 summary、request、completed、decisions、learned、next_steps、preferences、
   topic title/summary 与 transcript/source range 必须在任何 summary/topic/candidate/native-memory
   side effect 前完成 redaction 与统一 verdict。任一输入或输出命中时，summary 必须以 durable
   quarantine 记录落库，但未确认内容不得进入 topic、candidate、native-memory 或上下文注入。
7. `B-007`：quarantined summary 是“保留待审”而非“成功发布”或“永久删除”。状态必须记录目标
   summary、来源阶段（source 或 generated）、`pattern_id@version`、最低 source trust、证据范围和
   不含原文的时间/计数；重复处理同一 range 不得新建重复 summary 或重复调用模型。
8. `B-008`：SessionStart/context、Claude native memory、后续 observation/summary prompt、user-context
   extraction/recall 及任何其它 model-visible reader 必须通过同一 eligibility gate 读取 recent
   session summaries，并复扫 legacy 或当前可注入文本。未持有精确 acknowledgement 的任何命中
   summary 都必须 fail closed 地排除，同时返回其余安全输入并记录 error 级、可观测的 block；
   scanner、schema 或状态查询失败时也不得把该 summary 交给模型或衍生写入。
9. `B-009`：summary review 必须复用现有 governance 的 `acknowledge_pattern` 语义和审计事件，
   以显式 target kind 区分 memory 与 session summary；不得建立自动批准或互不相认的第二套 review
   queue。旧 CLI/MCP 调用未提供 target kind 时必须继续只作用于 memory。
10. `B-010`：非 dry-run acknowledgement 必须同时具备准确的 `pattern_id`、当前 pattern version、
    显式 reason、actor 与确认；ID/version 不匹配、pattern 已升级、目标不是 quarantine、项目不匹配
    或缺少任一授权证据时必须拒绝且不发生部分写入。
11. `B-011`：summary 被准确确认后才可重新具备注入资格；此前被阻断的 topic/candidate/native
    side effects 只能通过现有持久化 checkpoint 至多释放一次。释放出的 candidate 仍须独立应用
    `B-004`，acknowledgement 不能跨 artifact 或跨 pattern 自动继承。
12. `B-012`：并发 worker、重复 hook、重试、context load 与人工 review 交错时，同一
    `pattern_id@version` generation 的 quarantine/acknowledgement 转换必须原子且单调：
    `legacy_unscanned/safe -> quarantined -> acknowledged`；pattern version 升级可开启新的 quarantine
    generation，但必须保留旧 ack audit。陈旧 writer 不得覆盖较新的 generation/quarantine/ack，且
    一次 range 只能有一个 durable rollup 与一组 side-effect checkpoints。
13. `B-013`：`remem doctor` 与 `remem status` 必须分别显示 pattern set version、candidate 与
    summary quarantine 数、legacy-unscanned summary 数、source/generated 分类、context block 数和
    最近安全元数据。查询失败必须是 `Fail`/命令错误或带年龄和 warning 的显式 stale API 响应，
    不得显示 0 或当前成功状态。
14. `B-014`：quarantine、block、acknowledgement 和 side-effect release 的审计/log/status 只能包含
    稳定 ID、项目、阶段、pattern、version、trust、时间和计数；原始/生成内容最多提供再次 redaction
    后的有界 preview，默认输出不得包含载荷或 secret。
15. `B-015`：升级前的 session summaries 必须迁移为保守的 `legacy_unscanned`，不批量调用 LLM、
    不自动 acknowledgement。它们在首次注入前复扫：无命中可继续使用，命中或扫描失败按 `B-008`
    处理；既有 memory/candidate 的 GH-672 acknowledgement 语义保持兼容。
16. `B-016`：空 event range 继续产生既有 `EmptyRange` 结果；合法但所有结构化字段为空的 rollup
    仍须依据 summary 与 source range 判定。redaction 后所有生成字段均为空、缺失 required field 或
    输出无法解析时必须作为显式 extraction failure，不得写入伪安全的空 summary。
17. `B-017`：capture poisoning eval 必须从真实生产 capture/schema 边界开始，以确定性 fake
    extractor/rollup 驱动 observation-candidate 与 session-summary 两条路径，不依赖网络或 live LLM；
    instruction override、authority claim、opaque payload、中文、secret 混合载荷的 active memory
    数与 injected summary 数都必须为 0，且预期 quarantine/doctor/status 证据完整。
18. `B-018`：adversarial-policy suite 必须把 instruction injection、authority claim、opaque
    payload 和 benign quoted instruction 作为显式类别；恶意 case 的 policy leak rate 为 0，
    benign case 不得被当作 active poison 静默放行——若确定性模式命中，必须进入可人工确认的
    quarantine，并计入 false-positive/review 指标。
19. `B-019`：GH-852 或以后新增的 Claude/Codex 原生记忆、文件、MCP 或远程来源必须在创建 active
    artifact 前提供同一 source evidence/verdict 输入；不支持该契约的 adapter 必须 fail closed。
    GH-855 的实现和验收只使用当前已有 capture adapter，不能以 GH-852 未落地为阻塞理由。
20. `B-020`：本 spec 在缺少 issue 所引用研究报告的不可变证据时只能进入待审批状态；不得以
    issue 摘要、猜测的百分比或 agent 自行生成的替代报告满足 `spec_approval`。maintainer 接受等价
    一手证据时，批准记录必须指出该证据及 revision。
21. `B-021`：`legacy_unscanned` 只能由 schema migration 标记升级前已经存在的 summary row；迁移完成
    后的 rollup、`finalize_summarize` 及任何其它运行期 writer 都不得创建或恢复该状态。writer 必须在
    写入任何 derived summary/topic/candidate/native-memory 前取得完整、project/session/range 绑定的
    source evidence 并形成 combined verdict；证据缺失或不可验证时整个 derived write 原子失败、错误
    可见且规范 raw capture 保持可重试，禁止先写 generated output 再靠后续扫描“洗白”。
22. `B-022`：summary eligibility 必须在 `query_recent_summaries` 解码原始 row 后、正文参与
    self-diagnostic、cluster/session dedup、stale fallback、implicit query、hybrid retrieval、abstention、
    ranking 或任何其它派生选择之前完成。scanner、schema、row decode、状态或审计更新失败的 row 不得
    影响查询词、候选 ID、顺序或渲染；必须 fail closed 地排除并产生 error-visible 证据，安全 row 可
    继续处理。
23. `B-023`：`src/git_trace.rs` 暴露的 commit/session summary trace 及其 MCP `lookup_commit`、
    `commits_for_session` 输出属于 model-visible sink，必须复用同一 eligibility gate，并把每个 summary
    ID 同 commit 所属 project、link/session identity 精确绑定。跨项目或错误 ID、未确认命中以及
    scanner/schema/audit 失败均不得返回 summary 正文，且必须作为可见错误传播；不得以裸 SQL、空
    `summary` 或 warning-only fallback 隐藏失败。

## 验收标准

- [ ] 从 tool output 与 transcript 注入的英/中 instruction override、authority claim、opaque
      payload 和 secret 混合载荷完整经过 capture -> extraction -> candidate/summary -> context，
      active poison memory 与 injected poison summary 均为 0。
- [ ] 模型输出不含命中短语但 source evidence 命中的 laundering fixture 仍进入现有 quarantine，
      且未产生 auto-promoted memory、topic 或 native-memory side effect。
- [ ] quarantined summary 可用现有 governance action 的显式 summary target dry-run、拒绝错误 ack、
      接受准确 ack，并在并发/重试测试中只释放一次 checkpointed side effects。
- [ ] legacy summary 在所有 model-visible reader 前复扫；命中、scanner error、schema/query error
      都 fail closed 且可见，安全 legacy summary 保持兼容。
- [ ] 只有升级 fixture 的既有 row 会成为 `legacy_unscanned`；所有运行期 summary writer 在完整 source
      evidence 不可取得时返回错误、零 derived row/side effect，retry 不重复 LLM 且不能把状态回退为
      legacy/safe。
- [ ] poisoned/error summary 与“该 row 不存在”的对照在 implicit query、cluster/session dedup、hybrid
      retrieval、abstention、selected memory IDs/rank 上等价；其载荷不能成为 retrieval-steering signal。
- [ ] git trace 与 MCP commit tools 对 safe/精确 ack summary 保持兼容；未确认、跨 project/ID、scanner/
      schema/audit failure 均不返回正文并产生明确 tool/query error。
- [ ] doctor、CLI status 与 API status 的 fresh/stale/error case 均提供不含载荷/secret 的 poisoning
      计数和诊断。
- [ ] adversarial-policy 与新的 capture E2E eval 为确定性、离线且包含恶意/无害引用对照；相关
      baseline/report/artifact 可由仓库命令复现并通过 verifier。
- [ ] current contract `docs/specs/memory-poisoning-defense/`、README/architecture、schema migration、
      version surfaces 与实现同步，focused tests、migration/schema drift、eval gates、完整 Rust/JS/
      PR preflight 通过。
- [ ] 缺失研究报告由不可变 revision 或 maintainer 明确批准的等价证据补齐；随后才可授予
      `spec_approval` 与 `ready_to_implement`。

## 边界情况

### Boundary checklist

| 边界类别 | 结论 |
| --- | --- |
| Empty / missing input | covered: `B-003`, `B-005`, `B-016` |
| Error and failure paths | covered: `B-005`, `B-008`, `B-013`, `B-016`, `B-021`, `B-022`, `B-023` |
| Authorization / permission | covered: `B-009`, `B-010`, `B-020` |
| Concurrency / race / ordering | covered: `B-006`, `B-011`, `B-012`, `B-021`, `B-022` |
| Retry / repetition / idempotency | covered: `B-007`, `B-011`, `B-012`, `B-021` |
| Illegal state transitions | covered: `B-010`, `B-012`, `B-021` |
| Compatibility / migration | covered: `B-009`, `B-015`, `B-019`, `B-021` |
| Degradation / fallback | covered: `B-005`, `B-008`, `B-013`, `B-022`, `B-023` |
| Evidence and audit integrity | covered: `B-001`, `B-002`, `B-014`, `B-020` |
| Cancellation / interruption / partial completion | covered: `B-005`, `B-006`, `B-011`, `B-012` |

- 同一载荷同时命中多个 pattern：使用版本化 pattern table 的稳定优先级记录一个 primary match，
  eval 可记录完整命中集合；重跑不得随机改变 primary pattern。
- 无害文档引用包含精确危险短语：确定性 scanner 允许保守 quarantine，人工准确确认后恢复；不得
  为降低 false positive 而把引用语境默认视为安全。
- summary 已确认后 pattern set 升级：旧 version acknowledgement 不再满足新 match，重新阻断并
  要求新版本 review。
- context 在只读/损坏数据库上运行：无法验证或记录 summary 状态时不注入该 summary，并把错误
  放入现有 hook/context warning；不能因审计写失败而把内容放行。
- API status refresh 失败但存在短期 cache：只能返回标有 `stale=true`、生成时间、年龄和 warning
  的有界 stale 数据；超过既有最大 stale window 后返回错误。
- quarantine 发生在 raw archive drain 失败的同一次 rollup：两类失败证据都保留，retry 不重复
  模型调用，也不能先释放安全 side effects。
- migration 与旧 binary 交错：迁移后仍省略 verdict/试图写 `legacy_unscanned` 的 writer 必须被数据库
  invariant 拒绝并返回错误；不能用 default 状态制造新的 legacy row。
- eligibility 记录 quarantine/block audit 失败：该 row 仍在任何正文比较、dedup 或 retrieval signal
  前被排除，调用方收到错误；审计失败不能转成放行，也不能删除其它已验证安全 row。

## 发布说明

这是安全与可观测性增强，包含 session summary schema 迁移和 CLI/MCP governance 的向后兼容
扩展。发布说明必须告知用户：新版本可能把包含文档引用或命令示例的 summary/candidate 保守地
放入 quarantine；可通过 doctor/status 查看并使用现有 governance acknowledgement 审核，禁止
直接改数据库绕过。

spec-only PR 使用 `Refs #855`。本文件不构成 `spec_approval`、security approval、最终 review、
merge 或 release 授权；尤其在 `B-020` 的证据缺口关闭前不得进入实现。
