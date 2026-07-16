# Product Spec

## Linked Issue

GH-854

complexity: large

## 用户问题

remem 的 SessionStart 会在固定字符预算内注入 Core、Lessons、MemoryIndex、Sessions 等上下文。
当前 Core 已有自身的最低分规则，但 Lessons、MemoryIndex 和 Sessions 主要按已有顺序和分区预算
截取，没有共同的任务相关性门槛。结果是：与当前任务关系弱的历史内容仍可能占用输入预算，而真正
有用的少量内容被更多噪声稀释；当某个分区为空时，用户也难以区分“没有数据”“相关性不足”
“被 k 上限丢弃”和“被字符预算截断”。

GH-854 还要求用 `k in {1,3,5,10}` 的受控实验判断较小注入量是否改善真实编码任务效果。
目前 issue 引用的 `docs/research/agent-memory-optimization-research-2026-07.md` 不存在于
`origin/main`，仓库也没有现成的 SessionStart k sweep。外部数字、评分器、阈值、回归预算和
默认 k 因而不能由本 spec 推定，必须以可复现证据和 human `spec_approval` 冻结。

## 目标

- 对 SessionStart 的 Lessons、MemoryIndex 和 Sessions 建立“先相关、再少量”的共同选择契约。
- 在内容不足够相关时宁可少注入或不注入，不用低相关内容填满预算。
- 让每个候选项的最终命运以及空白、丢弃、截断和失败原因可见、可审计。
- 用当前生产基线和 `k in {1,3,5,10}` 的可复现实验比较任务效果、噪声、token、延迟和失败。
- 仅在预先批准的质量与回归预算内选择最小可接受 k，并保留独立的人类默认启用门禁。

## 非目标

- 不改变 Core、Preferences、Workstreams、错误提示或检索诊断的既有语义。
- 不用 GH-854 重新设计捕获、提取、数据库 schema、混合检索或全局 reranker。
- 不把 UserPromptSubmit 的词元重叠门槛或其他 issue 的 reranker 分数直接声明为可用的
  SessionStart 共同分数。
- 不把 golden 选择准确率替代真实 coding-bench 下游效果，也不以一次样本或事后挑选指标
  决定默认值。
- 不在 spec-only PR 中实施、运行 PoC、修改默认值、迁移数据、发布或合并。
- 不采纳缺失研究报告中的外部结论或 issue 引用数字，直到来源、适用性和复现证据获批。

## Behavior Invariants

1. `B-001`：GH-854 的受控分区闭集为 `Lessons`、`MemoryIndex`、`Sessions`；Core、
   Preferences、Workstreams 和错误/检索提示继续遵守既有契约，不能因为某个受控分区失败而被
   一并隐藏或重排。
2. `B-002`：相关性策略关闭时，三个受控分区的候选选择、正文顺序和既有 item/字符/总预算
   结果必须与变更前一致；新增诊断元数据可以显示 `off`，但不得伪装成已执行相关性筛选。
3. `B-003`：相关性策略启用时，每个受控候选必须先获得同一已批准量纲上的任务相关性分数；
   只有达到已批准门槛的候选才有资格进入全局 k 选择。
4. `B-004`：系统必须先应用相关性门槛，再从合格候选中最多选择 k 个唯一项，最后应用既有
   分区 item/字符预算和总字符预算；不得调换顺序来绕过任一约束。
5. `B-005`：当达到门槛的候选少于 k 个时，系统必须少注入或保持分区空白；不得用低于门槛、
   无分数或重复的内容回填预算。
6. `B-006`：同一查询、候选快照、策略版本和配置必须产生相同分数、顺序与决策；分数相同时
   使用稳定、公开且可复验的 tie-break，不能依赖哈希迭代或并发完成顺序。
7. `B-007`：当启用策略但任务上下文为空、过弱或无法形成有效查询时，三个受控分区必须
   `abstained` 并使用闭集原因说明；不得把“无法判断相关性”当作“全部相关”。非受控分区仍按
   `B-001` 输出。
8. `B-008`：当评分器不可用、超时、返回非法值或只完成部分候选时，本次三个受控分区必须
   整体 fail-closed，不得注入部分评分结果；用户可见状态和错误诊断必须区分该失败与正常空白。
9. `B-009`：缺失、越界或互相矛盾的启用配置必须被明确拒绝或进入可见错误状态；不得静默
   换用一个未经批准的评分器、阈值或 k 后继续成功。
10. `B-010`：每个受控候选的审计记录必须对应最终实际输出，包含稳定身份、分区、原始顺序、
    分数及评分器版本（若已评分）、门槛、最终状态和闭集原因；被总字符截断的项不能仍记录为
    `injected`。
11. `B-011`：SessionStart 状态必须区分 `off`、`applied`、`abstained` 和 `error`，并按受控
    分区显示合格、注入和各原因丢弃数量；“相关性不足而空白”与“有合格项但被预算截断”不得
    共享同一显示或原因。
12. `B-012`：若项目级审计写入失败，运行时不得把本次选择标记为完整审计成功；至少保留用户
    可见的选择状态并记录 error 级诊断，不能只 warning 后静默丢失证据。
13. `B-013`：并发 SessionStart 请求必须使用各自冻结的查询、候选快照和策略配置；一个请求的
    得分、取消、失败或预算消耗不得改变另一个请求的决策。
14. `B-014`：对同一冻结输入重试必须幂等；策略版本、门槛或 k 改变时必须形成新的选择身份，
    不能因旧的重复注入证据而错误复用先前决策。
15. `B-015`：请求被取消、超时或中断时不得发布半份受控选择或把未完成实验报告标为成功；
    重试从新的完整请求开始，先前的部分证据不能用于默认值批准。
16. `B-016`：k sweep 必须在同一预注册实验中包含当前生产基线以及 `k=1,3,5,10` 四个实验臂，
    并固定评分器、门槛、候选集、任务集、提交、runner/model/environment、运行次数和执行顺序；
    不得在查看结果后更换主要指标或排除失败样本。
17. `B-017`：每个实验臂必须报告任务完成率、memory helped/hurt、irrelevant injection、
    missing relevant、citation precision/recall、输入 token/字符、wall time、失败分类，以及各
    受控分区的合格/注入/丢弃原因；原始证据引用和配置身份必须足以独立复验汇总。
18. `B-018`：默认 k 只能从满足预先批准的任务质量、噪声、token、延迟和失败回归预算的实验臂
    中选择最小值；若没有实验臂满足全部预算，结论必须是“不启用”，不得选择表现最不差者。
19. `B-019`：golden 或确定性选择评测可以证明选择规则与相关项覆盖，但不能单独批准生产默认；
    缺失、失败或不可复验的 coding-bench 证据必须阻止默认启用声明。
20. `B-020`：评分和选择必须在本机、离线、有限资源内完成，只处理当前请求已获授权读取的内容；
    不得为 SessionStart 额外发送网络/LLM 请求，日志和实验报告不得泄露原始记忆正文或任务查询。
21. `B-021`：Claude Code 与 Codex 的 SessionStart 必须遵守同一策略版本和选择语义；存量数据
    无需迁移，旧配置未显式启用时按 `off` 处理。新增可观测字段不得让旧数据被误判为已评分。
22. `B-022`：在缺失研究来源、共同评分器与校准证据、精确阈值、实验回归预算、运行样本数和
    human `spec_approval` 任一项未解决时，策略必须保持默认关闭，issue 不得进入实现或默认启用
    完成状态。

## 验收标准

- [ ] 对当前生产基线可复现：Core 有自身门槛，而 Lessons、MemoryIndex、Sessions 没有共同
      任务相关性门槛，仓库没有 SessionStart k sweep。
- [ ] 开启策略后的选择符合“门槛 → 唯一项 k → 现有分区预算 → 总预算”，且不足 k 时不回填。
- [ ] 每个受控候选的最终状态与实际输出一致，空白、低相关、k 丢弃、分区预算和总截断可区分。
- [ ] 空查询、评分失败、非法配置、并发、重试、取消和审计失败都按对应 invariant fail-visible。
- [ ] 受控 sweep 比较生产基线与四个 k，产出可复验的任务效果、噪声、成本、延迟和失败证据。
- [ ] 只有在研究来源、评分契约、阈值、回归预算和样本规模获 human `spec_approval` 后，才可
      写 tasks 并进入 `ready_to_implement`；默认启用仍需后续独立批准。

## 边界情况

| 类别 | 结论 |
| --- | --- |
| Empty / missing input | covered: `B-005`, `B-007`, `B-009` |
| Error and failure paths | covered: `B-008`, `B-009`, `B-012`, `B-019` |
| Authorization / permission | covered: `B-020`, `B-022`；只读当前请求已授权内容，spec/default 均保留 human gates |
| Concurrency / race / ordering | covered: `B-004`, `B-006`, `B-013` |
| Retry / repetition / idempotency | covered: `B-014`, `B-016` |
| Illegal state transitions | covered: `B-002`, `B-009`, `B-018`, `B-022` |
| Compatibility / migration | covered: `B-001`, `B-002`, `B-021` |
| Degradation / fallback | covered: `B-005`, `B-007`, `B-008`, `B-019` |
| Evidence and audit integrity | covered: `B-010`, `B-011`, `B-012`, `B-016`, `B-017` |
| Cancellation / interruption / partial completion | covered: `B-008`, `B-015` |

特殊组合：即使评分器曾对部分候选成功，只要同次请求有超时/非法分数，`B-008 + B-015`
仍要求整体 fail-closed；即使某个 k 在事后看起来最好，只要预注册预算或可复验 coding-bench
证据缺失，`B-018 + B-019 + B-022` 仍要求默认关闭。

## 待 human `spec_approval` 冻结的问题

- 补齐并审阅 issue 引用的研究报告，或明确撤销该引用；外部结论必须有可核查来源和对 remem
  的适用性说明。
- 选择共同评分器、版本、特征与归一化方式，证明三个受控分区的分数可比较；不得默认复用
  UserPromptSubmit token overlap 或尚未合并的其他 issue 结果。
- 冻结阈值、候选投影、最大执行时间、非法/弱查询判定、闭集 reason codes 和稳定 tie-break。
- 预注册 coding-bench 的任务集、运行次数、reference model/runner/environment、主要指标，以及
  任务质量、噪声、token、延迟和失败的精确非回归预算。
- sweep 完成后由 human 明确批准默认 k 和是否启用；实验授权、implementation 授权、merge
  授权和 release 授权互不继承。

## 发布说明

本文件是 Draft 行为契约，不构成 `spec_approval`、PoC、`ready_to_implement`、merge 或 release
授权。当前默认保持关闭。未来若获批实现，先以可回滚的关闭模式交付诊断与实验能力；任何默认
启用必须引用完整 sweep 证据并通过独立 human gate。无需数据迁移；用户可见文档必须说明受控
分区、选择状态、空白/截断原因、配置兼容性和关闭方法。
