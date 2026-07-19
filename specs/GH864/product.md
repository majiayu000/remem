# Product Spec

## Linked Issue

GH-864

## 用户问题

真实的 `remem doctor` 与 exhausted extraction range 恢复暴露出四个相互独立但共同阻塞健康恢复的问题：

- 系统自己生成的 bounded transcript evidence 在持久化后重新校验时可能因尾部空白产生不同结果；
- Git branch/commit 探测可能无界等待，让 capture 或 rollup 卡死；
- 运维人员无法只重试或隔离一个明确的 extraction replay range；
- 合法语义的版本型 `topic_key` 会因点号等标点被拒绝，导致整个 rollup 失败。
- 已明确隔离的 range 没有受控恢复入口：range 308 当前为 `quarantined`，默认 exact retry 按既有
  合同正确拒绝，但运维人员即使恢复了 provider 认证也无法完成经确认的单 range 恢复。

用户需要可重复、可审计、不会误伤 sibling ranges 的精确恢复路径。修复必须保持现有批量命令兼容，
不能通过吞掉错误、放宽为空值或让 Git 子进程留在后台来伪装成功。

## 目标

- 让系统生成的脱敏、截断 transcript evidence 可稳定持久化并重复校验。
- 为 Git branch/commit 探测设置固定上限，超时后终止并回收子进程，同时保留可诊断错误。
- 为 retry/quarantine 增加 exact range ID 模式，并证明只改变目标 range。
- 复用统一 topic slug 规则规范化 `topic_key`，接受有意义的版本标点，同时对空结果继续 fail closed。
- 保持无 `--id` 时的现有批量行为、dry-run 语义和数据模型不变。
- 为已隔离 range 提供必须显式确认的 exact-ID 恢复路径，同时保持默认 retry 与批量 retry 不会选择隔离项。
- 为已归档且已隔离的单个 range 接通既有 `--include-archived` escape hatch，并提供只 claim 一个 replay
  task、显式选择 Claude profile 的 worker 运维路径。

## 非目标

- 不改变 extraction replay range 的 schema、最大尝试次数、普通 worker 排序/排空语义或失败分类。
- 不自动重试所有 exhausted ranges，不替用户选择应恢复的 range。
- 不改变 transcript evidence 的消息数、单消息字节数、总字节数、角色、脱敏或 exact-range 归属门禁。
- 不把 Git probe 失败解释为仓库成功探测；不新增 shell 调用、网络调用或 Git 配置写入。
- 不重新设计 topic/workstream 身份，也不更改 `slugify_for_topic` 的全局规则。
- 不声称代码合并即可让 range 308 成功；真实重放仍需要可用的 Claude profile 和运行时证据。
- 不自动解除 quarantine，不允许 batch 操作选择 quarantined/archived range，也不提供绕过 active-task
  或 terminal 门禁的通用强制开关。

## Behavior Invariants

1. **B-001**：对每条 transcript message，系统必须先执行现有敏感文本脱敏，再在 UTF-8 字符边界内
   应用现有单消息字节预算，最后移除截断结果的尾部空白。相同输入重复处理必须得到相同字节串。
2. **B-002**：总字节预算需要再次缩短最早消息时，必须使用与单消息相同的“UTF-8 安全截断 +
   尾部空白移除”规则，并按实际缩短后的字节数更新预算；不得持久化空消息。
3. **B-003**：B-001/B-002 不得放宽现有 evidence 校验：超过消息数/字节预算、非
   `user|assistant` 角色、未脱敏、空内容或不属于 exact rollup range 的事件仍必须返回错误。
4. **B-004**：branch/commit soft probe 以及真实 commit evidence 路径中
   `resolve_toplevel`、`detect_commit_metadata`、`resolve_commit_metadata` 发起的每次 Git 子进程调用
   最多等待 2 秒。超时时必须终止并回收该子进程；soft probe 返回“无探测结果”，required metadata
   返回带命令类别和可定位 cwd 的错误，既有 caller 继续按其 fail/skip 语义处理。stdout/stderr 必须在
   child 运行期间并发 drain；合法的大输出不得因填满 OS pipe 而被误判为 timeout。
5. **B-005**：Git 启动失败、`try_wait`/wait/kill/reap 失败必须以 error 级别暴露，不得 panic 或无限
   等待。spawn 成功后的任何错误分支都必须尝试 bounded best-effort kill/reap，并把 cleanup 失败附加
   到原 lifecycle error；不得通过 `?` 直接返回而跳过 child 清理。每个 Git probe 必须进入独立进程组，
   timeout/lifecycle error 必须有界终止整组，reader completion/join 也必须受 cleanup deadline 约束；
   后代进程持有 stdout/stderr 时不得让调用方无界等待。普通非零退出继续遵循现有 soft probe None /
   required metadata error 语义，不得伪造成 branch/commit。
6. **B-006**：所有上述 Git 命令必须继续使用参数数组而非 shell 字符串；cwd、branch 或 commit 内容不得
   被解释为额外命令、重定向或 shell 语法。
7. **B-007**：`list-extraction-ranges`、`retry-extraction-ranges` 与 `quarantine-extraction-ranges` 必须接受可选
   `--id <positive-i64>`。指定 `--id` 时，只有用户在命令行显式提供的 `--project` 或 `--limit`
   必须在参数解析阶段冲突；仅由 Clap 注入的默认 limit 不得让 `--id`-only 命令失败，也不得进入
   batch 路径。不指定 `--id` 时现有 project/limit 默认值与批量模式保持不变。exact list 必须返回目标
   range（包括 terminal `replayed`）及其 replay task ID/status/attempt/error 证据；不存在时非成功退出。
8. **B-008**：exact-ID dry-run 必须通过只读连接验证同一个 retryable predicate：目标存在、
   未 archived、状态为 `pending|failed`，且没有关联的 `pending|processing` replay task。
   不满足时命令必须非成功退出，且不得退回批量选择。
9. **B-009**：exact-ID retry 必须在一个事务中重新验证 B-008，并只为目标 range 建立或恢复
   idempotent replay task；exact-ID quarantine 必须在一个事务中只把目标 range 设为
   `quarantined` 并清理该 range 对应的 terminal failures。
10. **B-010**：exact-ID retry/quarantine 成功、失败、重复或并发竞争均不得改变任何 sibling range。
    若目标在 dry-run 后变为不可操作，实际执行必须报错或因数据库并发而失败，不得改选另一个 range。
11. **B-011**：未指定 `--id` 的批量 retry/quarantine 必须保留现有 oldest-first、project filter、
    limit、事务和返回计数语义；新增 exact 路径不得改变存量脚本的参数或输出含义。
12. **B-012**：rollup parser 必须先 trim 原始 `topic_key`，再复用现有
    `slugify_for_topic(..., 96)` 规则。示例 `v0.2-release-audit` 必须稳定得到
    `v0-2-release-audit`；重复标点必须按统一 slug 规则折叠。
13. **B-013**：缺失、trim 后为空或规范化后为空的 `topic_key` 必须返回明确解析错误。不得创建
    空 topic identity，也不得因规范化失败退回未经验证的原值。
14. **B-014**：同一原始 key 在相同版本和配置下必须得到稳定规范化结果。符合旧 parser grammar
    `[a-z0-9_-]+` 且至少包含一个 ASCII 字母或数字的既有合法 kebab-case/snake_case key 必须原样
    保留；其它输入进入共享 slug 规范化，避免把已持久化的 `foo_bar` 意外分裂成新 identity
    `foo-bar`，同时禁止 `---`、`___` 等纯标点 key 绕过 B-013。
15. **B-015**：实现提交必须同步所有发行版本面并记录 changelog；代码验证通过不等同于真实
    range 308 已恢复。关闭 GH-864 前还必须在可用 Claude profile 下执行 exact-ID 重放，等待 worker
    终态，并记录 exact list 的 range/task 结果与对应 replay task 的已脱敏 provider/profile 运行日志。
16. **B-016**：`retry-extraction-ranges` 必须接受可选 `--acknowledge-quarantine`，且该参数必须在
    参数解析阶段要求同时提供正数 `--id`。缺少确认时，B-008 的默认 predicate 不变；该参数不得用于
    list、quarantine 或任何 batch 选择。
17. **B-017**：仅当显式确认存在时，exact-ID dry-run 才可把 `quarantined` 视为候选状态；目标仍必须
    存在、未 archived 且没有关联的 `pending|processing` replay task。确认不能让 `requeued|replayed`、
    不存在、已归档或 active-task 目标通过，也不得回退到其它 range。
18. **B-018**：显式确认后的实际 retry 必须在一个事务中重新验证 B-017，只把目标 range 转为
    `requeued` 并建立或恢复其 idempotent replay task。确认路径的失败、重复和竞争不得改变 sibling
    ranges；未带确认的 exact retry 与所有 batch retry 的可选集合必须与既有行为完全一致。
19. **B-019**：range 308 的运维证据必须同时记录 quarantine 确认、成功的 Claude profile live check、
    exact dry-run、实际 retry、最终 range/task 状态及已脱敏 provider/profile 日志；任一步失败时 issue
    保持 open，且不得用直接数据库更新或批量 retry 代替。
20. **B-020**：`retry-extraction-ranges` 的 `--include-archived` 只能与正数 exact `--id` 和 `--dry-run`
    一起使用，且 archived `quarantined` 目标必须同时提供 `--acknowledge-quarantine`。缺少任一确认或尝试
    从 pending 命令执行写入时都必须失败；普通 exact、所有 batch、active-task、`requeued|replayed` 和
    不存在目标的集合不变。
21. **B-021**：带双重显式确认的 archived exact retry 只能由持有 worker singleton 的 exact worker 执行；
    它必须在同一事务中重新验证目标 ID、状态和无 active replay task，清除该目标的 archive marker、只
    requeue 该目标并立即取得该 replay task 的 exact lease，事务提交前不得暴露可由 daemon claim 的
    pending task。失败、重复、竞争或确认不完整不得修改目标或 sibling。
22. **B-022**：worker 必须提供只恢复一个正数 replay range ID 的 `--once` 模式；它在任何写入前取得
    worker singleton，持锁原子执行 B-020/B-021，并以与普通 claim 相同的 pending/到期 predicate 只
    process 已 claim 的 task。`--profile` 在写入前通过现有 resolver 验证并只用于该 task；覆盖完整 range
    的 done 正常提交，partial coverage、defer/wait/timeout/provider error 等非成功结果必须把 task 与 range
    恢复为 archived quarantine。exact
    owner 的过期 lease 也必须归档隔离而非普通 requeue，保证中断后不会被 daemon 以默认 profile claim。
    锁被 daemon 持有、task 未到期、缺失或竞争时失败，且不回退到全局 claim、maintenance、job、backfill
    或第二个 task。普通 `remem worker [--once]` 行为保持不变。

## 验收标准

- [x] trailing-whitespace 边界 fixture 证明 transcript evidence 首次生成与持久化重验一致。
- [x] 单消息和总预算路径覆盖 UTF-8、空结果、尾部空白与现有脱敏门禁。
- [x] Git 超时 fixture 证明 soft probe 与真实 commit metadata 路径共享 2 秒 executor；timeout child
      和同进程组后代被终止，`try_wait` 错误分支尝试 cleanup，reader completion 有界，启动和回收错误
      可见；超过 OS pipe buffer 的合法 stdout/stderr fixture 成功且不误报 timeout。
- [x] CLI parser 覆盖三个 extraction-range 命令的 `--id`-only、非正 ID、显式 project/limit 冲突、
      隐式默认 limit 不冲突及无 ID 的兼容模式；exact list 可查询 terminal replay/task 证据。
- [x] 两个 sibling ranges fixture 证明 exact retry 和 exact quarantine 只改变目标 ID。
- [x] `v0.2-release-audit`、既有 kebab-case/snake_case key、重复标点及纯标点 key 均有 parser 测试。
- [x] `cargo fmt --check`、`cargo check --locked`、focused tests、`cargo test --locked --quiet`、
      Clippy、插件版本同步与 PR preflight 通过。
- [x] 维护者对 Git 子进程生命周期和 exact-range DB 事务完成安全/正确性审核（见
      [GH-864 维护者审查记录](https://github.com/majiayu000/remem/issues/864#issuecomment-5006885226)）。
- [x] Claude profile 可用后，range 308 通过 exact-ID 路径重放并把结果记录在 GH-864。
- [x] CLI parser 证明 `--acknowledge-quarantine` 缺少 `--id` 时失败；DB 双-range fixture 证明只有显式
      确认的 quarantined 目标被 requeue，默认 exact 与 batch 均继续跳过 quarantine。
- [x] README 记录 exact-ID list/retry/quarantine 示例，failure-lifecycle PRODUCT/TECH 同步精确恢复合同。
- [x] archived quarantine fixture 证明 pending 命令只允许双确认 dry-run，且只有 exact worker 可写恢复；
      exact worker fixture 证明持锁后才写入、同一事务 requeue+claim 目标 task、保留 retry readiness、使用
      显式 profile，并在非成功或 exact lease 过期后重新归档而不暴露给 daemon。

## 边界情况

| Category | Verdict |
| --- | --- |
| Empty / missing input | covered: B-002, B-007, B-013 |
| Error and failure paths | covered: B-003, B-005, B-008, B-013 |
| Authorization / permission | covered: B-008, B-015, B-016, B-017, B-019, B-020, B-022；本地确认不可替代真实 provider 认证 |
| Concurrency / race / ordering | covered: B-009, B-010, B-018, B-021, B-022 |
| Retry / repetition / idempotency | covered: B-001, B-009, B-010, B-014, B-018, B-021 |
| Illegal state transitions | covered: B-008, B-009, B-010, B-016, B-017, B-018, B-020, B-021, B-022 |
| Compatibility / migration | covered: B-011, B-014, B-018, B-020, B-022；无 schema migration |
| Degradation / fallback | covered: B-004, B-005, B-008, B-013, B-017, B-019, B-020, B-022 |
| Evidence and audit integrity | covered: B-003, B-008, B-015, B-016, B-019, B-021, B-022 |
| Cancellation / interruption / partial completion | covered: B-004, B-005, B-009, B-018, B-019, B-021, B-022 |

## 发布说明

该修复作为一个 patch release 发布。说明应列出四项用户可见修复、exact-ID 命令示例和 Git probe
2 秒上限。升级不需要数据库迁移；回滚版本不会删除 replay ranges，但会失去 exact-ID list/retry/quarantine CLI、
稳定截断和 topic key 规范化能力。真实 range 308 的恢复证据属于运维收口，不应写成所有用户都会
自动恢复的发布承诺。
对于已隔离的单 range，发布说明必须展示显式确认参数，并明确默认 exact/batch retry 仍跳过 quarantine。
对于已归档的隔离 range，说明必须展示额外的 `--include-archived` 和 exact worker/profile 命令，并明确
这些参数不会扩展普通 worker 或 batch retry。
