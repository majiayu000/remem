# Product Spec

## Linked Issue

GH-818

## 用户问题

通用 jobs queue 目前不能可靠证明两件事：完成、重试或耗尽更新是否仍由当前 lease
owner 持有，以及两个并发进程是否只创建了一份相同工作。错误 owner 或过期 lease 的
状态更新可能没有改变任何行却被当作成功；两个独立连接也可能同时通过去重检查并各自
插入一条 active job。

这会让已经发生副作用的工作仍停留在 `processing`，随后被再次执行，也会让 Compress、
Dream 或 CompileRules 在并发调度时重复运行。worker 的成功日志、`remem status` 和
`remem doctor` 因而可能与数据库中的真实执行历史分叉，降低用户对后台记忆维护结果的
信任。

## 目标

- 让所有 lease-owned 完成、重试和终止转换在 lease 已失效时明确失败，且不改变原行。
- 让 active job 去重成为跨进程、跨连接都成立的数据库行为契约。
- 保持 Dream 的项目级合并语义和 CompileRules 的单一 pending successor 语义。
- 让迁移、worker 日志、`remem status` 和 `remem doctor` 如实呈现冲突与失败，不把未发生
  的状态转换显示为成功。
- 兼容已有数据库及其历史 job 记录，不通过静默删除掩盖存量重复。

## 非目标

- 不重新设计 `extraction_tasks`、其 lease 模型或 replay 生命周期。
- 不恢复 legacy Summary jobs，也不恢复已退休的 Summary 执行路径。
- 不用进程内 mutex、单 worker 假设或调用方自律代替跨进程数据库不变量。
- 不改变 Compress、Dream 或 CompileRules 的业务副作用、调度优先级、重试上限或失败
  retention/archiving 策略。
- 不借此重构 worker scheduler、failure taxonomy、doctor/status 的其他统计口径或旧
  observation queue。

## Behavior Invariants

1. `B-001`：任何由 job lease owner 发起的完成、回到 pending 重试、永久失败或重试耗尽
   转换，只有在目标 job 仍为 `processing`、owner 与调用方预期值相同且 lease 尚未过期
   时才可成功；成功必须恰好转换一行。
2. `B-002`：目标 job 不存在、已不在 `processing`、owner 不匹配、lease 已过期或已被
   其他 worker 回收时，转换必须返回错误并保持该 job 的状态、owner、lease、attempt、
   error 和时间字段不变，不得把零行更新解释为幂等成功。
3. `B-003`：lease-owned 转换失败必须提供足以定位竞争的诊断：job id、expected owner，
   以及目标存在时的 current state、current owner 和 lease expiry；无法取得当前行时必须
   明确报告 missing，而不是构造成功结果。若底层转换报告超过一行，必须按不变量破坏
   处理。
4. `B-004`：除下列显式例外外，同一 ordinary job identity 在任意时刻最多存在一条
   `pending|processing` 记录；identity 继续由 host、job type、project 和“缺失 session
   与空 session 等价”的 session identity 共同决定。`done`、`failed` 和已归档历史不阻止
   后续新工作。
5. `B-005`：Dream 保持现有项目级特殊 identity：同一 project 的 active Dream 在不同
   host 之间仍合并为一份工作；已有 pending Dream 只有收到非空且不同的 incoming profile
   时，才以该请求替换 payload/host，并允许降低 priority。空 profile 或与当前相同的 profile
   不得替换 payload/host 或降低 priority；processing Dream 的 payload 不得被后续请求改写。
   recent-done cooldown 抑制语义保持不变，并发竞争不得把“合并到已有工作”错误报告成
   “新建工作”。
6. `B-006`：CompileRules 保持现有 successor 例外：同一 project 最多有一条
   `processing` CompileRules 和一条 `pending` successor；当 processing predecessor 存在
   时，新变化可保留一个 pending successor，但该 successor 在 predecessor 离开
   `processing` 前不可被 claim。任意并发次数都不得产生第二个 pending successor 或第二个
   processing job。predecessor 的 retry 或 expired-lease recovery 与 successor 相遇时，必须
   收敛到同一个 pending successor：不得因 identity 冲突导致约束错误、队列冻结、来源错误/
   审计证据丢失或无关 job 被阻塞。worker 本次执行失败必须精确增加一次 `attempt_count`；仅
   expired-lease recovery 时保持当前计数；两者都不得为阻止重试而伪造 exhausted。worker 日志
   不得泄露原始错误，尚待执行的工作始终由恰好一个 pending successor 表示。
7. `B-007`：两个独立数据库连接或进程并发 enqueue 同一 ordinary identity 时，调用结果
   必须收敛到同一个 canonical job id，最终 active row 数为一；调用方不得收到未持久化的
   job id 或把约束竞争当作无关数据库错误静默丢弃。
8. `B-008`：一个 CompileRules 正在 processing 时，两个独立连接并发 enqueue successor
   必须返回同一个 pending successor id，最终状态精确为一条 processing 加一条 pending；
   没有 processing 时的并发首次 enqueue 也只能产生一条 pending。
9. `B-009`：升级前已经存在重复 active rows 的数据库必须能够迁移。迁移必须按稳定规则
   保留每个允许 active slot 的 canonical row；对重复 pending Dream，结果必须等价于按迁移
   规定的稳定顺序串行应用 `B-005`：只有非空且不同的后续 profile 替换 payload/host 并可
   降低 priority，空 profile 或相同 profile 不替换也不降低，processing Dream payload 不被
   改写。其余重复记录不得静默删除或伪装为成功，必须保留可查询历史并带有明确的迁移冲突
   诊断，同时保持每条 redundant row 的真实 `attempt_count`；不得伪造 exhausted，shared
   stats/status/doctor 也不得把 migration conflict 计作 exhausted retry。迁移后的 active 状态
   必须满足 `B-004` 至 `B-006`。任何 `processing` row 的 NULL lease expiry 必须在 survivor
   selection 前按 expired 处理，不能留下既不可恢复也不可观测的永久占槽 row。
10. `B-010`：迁移兼容不得覆盖既有 terminal job 的 state、attempt、error、failure class、
    archive marker 或时间证据；再次检查同一已迁移数据库不得选择不同 canonical row、
    复活历史工作或创建额外工作。
11. `B-011`：worker 只有在 lease-owned terminal transition 已成功后才能记录
    `done id=...` 或等价成功事件。转换失败必须以 error 级别暴露并向调用链返回失败；
    不得同时记录或返回会让调用方误以为 terminal transition 已成功的信号。
12. `B-012`：`remem status`、`remem status --json` 和 `remem doctor` 必须以持久化状态为
    准：转换失败后仍为 processing 的 job 继续显示为 processing，并在 lease 过期后显示为
    stuck；迁移产生的可操作失败继续进入 failure lifecycle。auto-recovery 与同一 active
    identity 相遇时必须逐条独立处理，保留来源 failure/audit 证据并把待执行工作收敛到
    canonical active job；来源 job 保持 failed/permanent、保留真实 retry attempt evidence，
    且不会再次进入自动重试。一条 collision 不得回滚其他无关 recovery。任何入口都不得仅凭
    worker 曾执行副作用或尝试完成就显示成功。
13. `B-013`：legacy Summary jobs 保持 retired。升级和新 enqueue 都不得为了满足新去重
    规则而重建、重试或转换为新的 Summary 工作；failure auto-recovery 的 candidate query
    必须排除 Summary，逐 row guard 对意外输入也必须返回明确的 retired/skipped 结果，并保持
    该 row 的全部 persisted audit fields 与 recovery counters 不变。既有 terminal Summary 历史继续可查询。
14. `B-014`：数据库 busy/locked、enqueue 冲突后的 canonical row 不可读取、迁移无法安全
    判断 canonical row，或状态诊断读取失败时，操作必须明确失败并保留原状态；不得降级为
    “假定已去重”或“假定已完成”。

## 验收标准

- [ ] wrong owner、expired lease、reclaimed lease、missing job 和 already-terminal job 的
      完成/重试/耗尽场景均返回错误，错误包含 `B-003` 所需诊断，且原 row 不变。
- [ ] 两连接并发测试证明 ordinary identity 最终只有一条 active row，两个调用方得到同一
      canonical job id。
- [ ] Dream 的跨 host inflight 合并、pending profile/priority 更新、recent-done cooldown 和
      并发 disposition 均保持现有可观察行为；fixture 证明仅非空且不同的 profile 替换
      pending payload/host 并可降低 priority，空或相同 profile 不替换、不降低，processing
      payload 不被改写。
- [ ] 两连接 CompileRules 测试分别证明首次 enqueue 最多一条 pending，以及 processing
      存在时最多保留同一个、且在 predecessor 完成前不可 claim 的 pending successor；retry
      与 expired-lease recovery 碰撞均收敛为恰好一个 pending successor，同时保留错误/审计
      证据且不阻塞无关 job。
- [ ] migration fixture 含 ordinary 重复、CompileRules 重复、空/相同/不同 profile 的 Dream
      pending 重复、processing Dream、terminal history 和已归档 failure；Dream 结果精确复现
      `B-005` 的串行行为，升级后 active 不变量成立，历史与冲突诊断仍可查询，再次检查不改变
      结果；低于 `max_attempts` 的 redundant rows 保持原始 attempt count，且 shared stats 不把
      migration conflict 计作 exhausted；NULL-expiry processing fixture 在迁移后可被既有 stuck
      统计与 expired recovery 发现。
- [ ] worker 测试证明 transition error 不产生日志成功事件；doctor/status fixture 证明其
      展示数据库真实 processing/stuck/failed 状态。
- [ ] failure auto-recovery fixture 证明 active identity collision 保留来源 failure/audit
      证据与真实 attempt count、收敛到 canonical work、不会重复自动重试，并且一条 collision
      不回滚无关 recoveries；普通 batch 排除 legacy Summary 而继续恢复无关 ordinary row，
      直接调用逐 row seam 时也明确 skipped-retired、保持 Summary 全部字段及 counters 不变；
      failure lifecycle actionable/archive 口径和 `extraction_tasks` 行为的回归测试保持通过。

## 边界情况

- `session_id` 缺失与空字符串继续按同一 identity 处理；其他 host、job type、project 或
  非空 session 仍是不同 ordinary identity。
- lease 在副作用完成后、terminal transition 前恰好过期：旧 worker 必须失败，不能覆盖
  已回收或可回收状态；是否重试副作用由既有失败/recovery 契约决定。
- 两个 caller 同时首次 enqueue、一个 caller enqueue 而另一个 claim、或两个 caller 同时
  enqueue CompileRules successor：最终状态都必须满足对应 active 数量上限；processing
  predecessor 存在时 successor 不可提前 claim。
- CompileRules predecessor retry 或 expired-lease recovery 恰好与 pending successor 相遇：
  二者必须合并为恰好一个可继续执行的 pending successor，来源错误/审计记录仍可查询；该
  project 的 collision 不得冻结队列或阻止其他 project/job 的 recovery。
- Dream 并发请求携带空、相同或不同 profile/priority：不得产生两条 active Dream；仅非空且
  不同的后续 profile 可替换 pending payload/host 并降低 priority，空或相同 profile 不得
  替换/降低，processing payload 始终不改写，并保留可解释的 disposition。
- failure auto-recovery batch 中只有部分 rows 与 active identity 碰撞：每个来源 row 的
  failure/audit 证据必须保留，碰撞项收敛到 canonical work，其他 rows 仍独立恢复成功。
- 存量重复中同时存在多个 processing owner 或无法无损决定 Dream payload：迁移必须保留
  证据并明确标记需要人工核对，不能任意删除、静默合并副作用历史或自动声称修复完成。
- 数据库只读、busy/locked、schema 不完整或迁移中断：操作失败且事务回滚；旧库保持可重试
  迁移的完整状态。
- 权限、网络、loading 和 accessibility：该能力是本地后台数据库契约，不新增授权、网络、
  交互式 loading 或 UI 可访问性状态，因此不适用。

## 开放问题

- 存量组内有多个 processing owner 时，canonical survivor 的稳定优先级以及其余 rows 的
  failure class/人工恢复入口，需要 maintainer 在 Tech Spec 中确认；无论选择为何，都必须
  满足 `B-009` 至 `B-012` 的可见历史与 fail-closed 约束。
- 存量重复 Dream 的稳定重放顺序由 Tech Spec 固定；无论采用何种稳定排序，其结果必须逐条
  复现 `B-005` 的串行 profile/payload/host/priority 语义，不能把空或相同 profile 当作更新。

## 发布说明

该变更需要数据库迁移。正常且无重复的既有数据库应自动升级，无需用户手工清理；发现
存量冲突时必须在日志、`remem status` 或 `remem doctor` 中给出可操作且不泄露 payload/
凭据的诊断。发布说明应明确：active job 去重现在跨进程成立、过期或错误 owner 的完成不再
被接受、Dream/CompileRules 特殊语义保持不变、legacy Summary 不会恢复。

本文件仅定义产品契约，不授予实现权限。`readiness_label`、`spec_approval`、最终 PR review、
merge 和 release 仍是 human gates；在 maintainer 完成 spec approval 并进入
`ready_to_implement` 前不得开始实现。
