# Product Spec

## Linked Issue

GH-864

## 用户问题

真实的 `remem doctor` 与 exhausted extraction range 恢复暴露出四个相互独立但共同阻塞健康恢复的问题：

- 系统自己生成的 bounded transcript evidence 在持久化后重新校验时可能因尾部空白产生不同结果；
- Git branch/commit 探测可能无界等待，让 capture 或 rollup 卡死；
- 运维人员无法只重试或隔离一个明确的 extraction replay range；
- 合法语义的版本型 `topic_key` 会因点号等标点被拒绝，导致整个 rollup 失败。

用户需要可重复、可审计、不会误伤 sibling ranges 的精确恢复路径。修复必须保持现有批量命令兼容，
不能通过吞掉错误、放宽为空值或让 Git 子进程留在后台来伪装成功。

## 目标

- 让系统生成的脱敏、截断 transcript evidence 可稳定持久化并重复校验。
- 为 Git branch/commit 探测设置固定上限，超时后终止并回收子进程，同时保留可诊断错误。
- 为 retry/quarantine 增加 exact range ID 模式，并证明只改变目标 range。
- 复用统一 topic slug 规则规范化 `topic_key`，接受有意义的版本标点，同时对空结果继续 fail closed。
- 保持无 `--id` 时的现有批量行为、dry-run 语义和数据模型不变。

## 非目标

- 不改变 extraction replay range 的 schema、最大尝试次数、worker 调度或失败分类。
- 不自动重试所有 exhausted ranges，不替用户选择应恢复的 range。
- 不改变 transcript evidence 的消息数、单消息字节数、总字节数、角色、脱敏或 exact-range 归属门禁。
- 不把 Git probe 失败解释为仓库成功探测；不新增 shell 调用、网络调用或 Git 配置写入。
- 不重新设计 topic/workstream 身份，也不更改 `slugify_for_topic` 的全局规则。
- 不声称代码合并即可让 range 308 成功；真实重放仍需要可用的 Claude profile 和运行时证据。

## Behavior Invariants

1. **B-001**：对每条 transcript message，系统必须先执行现有敏感文本脱敏，再在 UTF-8 字符边界内
   应用现有单消息字节预算，最后移除截断结果的尾部空白。相同输入重复处理必须得到相同字节串。
2. **B-002**：总字节预算需要再次缩短最早消息时，必须使用与单消息相同的“UTF-8 安全截断 +
   尾部空白移除”规则，并按实际缩短后的字节数更新预算；不得持久化空消息。
3. **B-003**：B-001/B-002 不得放宽现有 evidence 校验：超过消息数/字节预算、非
   `user|assistant` 角色、未脱敏、空内容或不属于 exact rollup range 的事件仍必须返回错误。
4. **B-004**：Git branch 与 commit probe 的每次子进程调用最多等待 2 秒。超时时必须终止并回收
   该子进程，调用方得到“无探测结果”，且 error 日志包含 probe 类别和可定位的 cwd 上下文。
5. **B-005**：Git 启动失败、wait/kill/reap 失败必须以 error 级别暴露，不得 panic、无限等待或遗留
   子进程。普通的非零 Git 退出继续表示“当前信息不可用”，不得伪造成 branch/commit。
6. **B-006**：Git probe 必须继续使用参数数组而非 shell 字符串；cwd、branch 或 commit 内容不得
   被解释为额外命令、重定向或 shell 语法。
7. **B-007**：`retry-extraction-ranges` 与 `quarantine-extraction-ranges` 必须接受可选
   `--id <positive-i64>`。指定 `--id` 时，只有用户在命令行显式提供的 `--project` 或 `--limit`
   必须在参数解析阶段冲突；仅由 Clap 注入的默认 limit 不得让 `--id`-only 命令失败，也不得进入
   exact-ID 执行路径。不指定 `--id` 时现有 project/limit 默认值与批量模式保持不变。
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
    `[a-z0-9_-]+` 的既有合法 kebab-case/snake_case key 必须原样保留；只有旧 grammar 会拒绝的
    输入才进入共享 slug 规范化，避免把已持久化的 `foo_bar` 意外分裂成新 identity `foo-bar`。
15. **B-015**：实现提交必须同步所有发行版本面并记录 changelog；代码验证通过不等同于真实
    range 308 已恢复。关闭 GH-864 前还必须在可用 Claude profile 下执行 exact-ID 重放并记录结果。

## 验收标准

- [ ] trailing-whitespace 边界 fixture 证明 transcript evidence 首次生成与持久化重验一致。
- [ ] 单消息和总预算路径覆盖 UTF-8、空结果、尾部空白与现有脱敏门禁。
- [ ] Git 超时 fixture 证明子进程在 2 秒上限内被 kill/reap，启动和回收错误可见。
- [ ] CLI parser 覆盖 `--id`-only、非正 ID、显式 project/limit 冲突、隐式默认 limit 不冲突及无 ID 的兼容模式。
- [ ] 两个 sibling ranges fixture 证明 exact retry 和 exact quarantine 只改变目标 ID。
- [ ] `v0.2-release-audit`、既有 kebab-case/snake_case key、重复标点及纯标点 key 均有 parser 测试。
- [ ] `cargo fmt --check`、`cargo check --locked`、focused tests、`cargo test --locked --quiet`、
      Clippy、插件版本同步与 PR preflight 通过。
- [ ] 维护者对 Git 子进程生命周期和 exact-range DB 事务完成安全/正确性审核。
- [ ] Claude profile 可用后，range 308 通过 exact-ID 路径重放并把结果记录在 GH-864。

## 边界情况

| Category | Verdict |
| --- | --- |
| Empty / missing input | covered: B-002, B-007, B-013 |
| Error and failure paths | covered: B-003, B-005, B-008, B-013 |
| Authorization / permission | covered: B-008, B-015；本地 CLI 无新增权限模型，但真实 provider 认证不可绕过 |
| Concurrency / race / ordering | covered: B-009, B-010 |
| Retry / repetition / idempotency | covered: B-001, B-009, B-010, B-014 |
| Illegal state transitions | covered: B-008, B-009, B-010 |
| Compatibility / migration | covered: B-011, B-014；无 schema migration |
| Degradation / fallback | covered: B-004, B-005, B-008, B-013 |
| Evidence and audit integrity | covered: B-003, B-008, B-015 |
| Cancellation / interruption / partial completion | covered: B-004, B-005, B-009 |

## 发布说明

该修复作为一个 patch release 发布。说明应列出四项用户可见修复、exact-ID 命令示例和 Git probe
2 秒上限。升级不需要数据库迁移；回滚版本不会删除 replay ranges，但会失去 exact-ID CLI、
稳定截断和 topic key 规范化能力。真实 range 308 的恢复证据属于运维收口，不应写成所有用户都会
自动恢复的发布承诺。
