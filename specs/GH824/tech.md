# Tech Spec

Status: Draft，等待人工批准；不得据此开始实现
Date: 2026-07-23

## Linked Issue

GH-824

- Epic: GH-821
- Depends on: GH-823
- Real-host gate: GH-822

## Product Spec

[`product.md`](./product.md)

## Planned Changes Manifest

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 824,
  "complete": true,
  "paths": [
    "README.md",
    "docs/ARCHITECTURE.md",
    "docs/specs/README.md",
    "specs/GH824/product.md",
    "specs/GH824/tech.md",
    "specs/GH824/tasks.md",
    "src/cli/types.rs",
    "src/doctor.rs",
    "src/doctor/cursor_install.rs",
    "src/doctor/cursor_install/tests.rs",
    "src/doctor/environment.rs",
    "src/doctor/report.rs",
    "src/doctor/types.rs",
    "src/install.rs",
    "src/install/config.rs",
    "src/install/cursor_config/mod.rs",
    "src/install/cursor_config/plan.rs",
    "src/install/cursor_config/schema.rs",
    "src/install/cursor_config/tests.rs",
    "src/install/cursor_config/writer.rs",
    "src/install/host.rs",
    "src/install/hosts/cursor.rs",
    "src/install/hosts/mod.rs",
    "src/install/paths.rs",
    "src/install/runtime.rs",
    "src/install/tests.rs",
    "src/runtime_config.rs",
    "tests/install_status.rs"
  ],
  "spec_refs": [
    "specs/GH824/product.md",
    "specs/GH824/tech.md"
  ]
}
-->

该清单把新 Cursor whole-document parser/plan/secure writer 和 doctor classifier 拆到专用
模块，避免继续扩大已接近文件上限的 `src/install/tests.rs`、
`src/doctor/environment.rs` 与 `src/doctor/tests.rs`。`src/install/config.rs` 只为复用现有
POSIX `shell_quote` 暴露最小可见性；不改变 Claude/Codex builder。若实现证明需要清单外路径，
必须先更新 manifest 并重新取得 exact-head 人工批准，不能在 implement gate 后临时扩 scope。

## 阻塞与授权边界

本 packet 可以在 `ready_to_spec` 阶段包含 `tasks.md`，用来记录依赖、文件 ownership、
验证命令和人工门顺序；它不授权写 runtime。所有 implementation task 必须同时等待：

1. GH-822/PR #914 exact head
   `c0802c42c3fc22770aecb0b7b2eec88f117f795c` 的 real-host PoC evidence 获准采用，
   并由 GH-823 amendment 固定 managed event/payload 与逐 capability `effective` 边界；
2. GH-823 spec 获批准且 canonical `cursor` identity、closed-set hook-host parser 与
   hook contract 已在 runtime 中实现；GH-824 的安装实现负责自己的 `hosts.cursor`
   defaults/normalization、receipt 与清理合同；
3. GH-824 product/tech 获人工批准，issue 进入 `ready_to_implement`，implementation
   route gate 返回 `allowed`。

任一门未满足时只能维护 spec/task planning evidence，不得开始、部分预写或以 feature
flag 隐藏 Cursor runtime 实现。

## Codebase Context

以下锚点已在 merge `origin/main` 后的本 PR worktree（base
`f612b4a1ec4558ed6d2df85699cefb42109bdf7c`）逐项确认。

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Target enum | `src/install/host.rs:7` | `InstallTarget` 只有 `Auto/Claude/Codex/All`；`InstallHost` 把 MCP 与 hooks 分成独立方法 | 需要增加 Cursor，同时避免沿用“先 MCP 后 hooks”造成 partial write |
| Host resolution | `src/install/hosts/mod.rs:15` | `Auto` 过滤 `is_available()`，`All` 返回全部已知 host | Cursor target 与 detected 语义入口 |
| Install orchestration | `src/install/runtime.rs:22`、`src/install/runtime.rs:95`、`src/install/runtime.rs:101` | dry-run 直接取 plan；real install 先 materialize runtime hosts，再逐 host 先 `install_mcp` 后 `install_hooks` | Cursor 需要 host-level preflight/staged apply；不能复用两次独立写 |
| Runtime store readiness | `src/install/runtime.rs:74`、`src/install/runtime.rs:99` | real install 当前先调用 `ensure_runtime_store_ready()` 创建/迁移 key+database，再 materialize host config，之后才进入 host install | Cursor/All/Auto-selected Cursor 必须先完成所有 Cursor preflight；malformed/collision/unsupported Cursor 不能在首次运行留下 store/config/token 副作用 |
| Runtime mapping | `src/install/runtime.rs:434` | 未识别 install host 映射为 `unknown` | GH-824 必须把 GH-823 的 canonical `cursor` identity 显式映射为安装 host，禁止 `hosts.unknown` |
| Uninstall orchestration | `src/install/runtime.rs:442` | `Auto` 转 `All`；逐 host 先 MCP 后 hooks，dry-run 只显示 primary path | Cursor 必须显示/校验两个路径并走同一事务协调器 |
| Generic JSON I/O | `src/install/json_io.rs:5`、`src/install/json_io.rs:15` | parse JSON 后 pretty-print whole document，单文件使用 atomic writer | 可以承诺语义保留，不能承诺 byte preservation；当前 writer 的新文件 temp 权限不满足 secret-before-write 要求 |
| Atomic writer | `src/atomic_file.rs:16`、`src/atomic_file.rs:30`、`src/atomic_file.rs:60` | `create_new` temp，先写内容，再在 `:69` 复制现有权限，随后 fsync/rename/parent sync | MCP foreign env 可能含 secret；Cursor writer 必须在 `write_all` 前设权限，并提供可注入的 staged/rollback failure tests |
| Existing JSON ownership | `src/install/config.rs:139`、`src/install/config.rs:152` | `is_remem_hook` 接受 `cmd.contains(bin) || cmd.contains("remem")` 并据此删除 | Cursor 禁止复用；需要精确 managed shape + 有限 legacy allowlist |
| Existing host implementations | `src/install/hosts/claude.rs:33`、`src/install/hosts/codex.rs:31` | MCP 与 hooks 各自 read/mutate/write，shape 容器有些错误会被静默跳过 | Cursor 使用独立严格 parser/plan/apply，不扩散现有宽松合同 |
| Paths | `src/install/paths.rs:4` | 只集中定义 Claude/Codex 路径 | 增加用户级 Cursor 两路径 helper，不增加 project path writer |
| Runtime config | `src/runtime_config.rs:103`、`src/runtime_config.rs:170` | `ensure_config_for_hosts` materialize host；`normalize_host` 对未知值 passthrough | GH-824 定义并实现 `hosts.cursor` defaults/normalization、receipt 与 cleanup；只消费 GH-823 canonical identity，不允许 fallback 到 unknown |
| Doctor host probes | `src/doctor/environment.rs:49`、`src/doctor/environment.rs:59`、`src/doctor/environment.rs:90` | probe 只有 name/hooks/mcp paths；present host 分别做 hooks/MCP check | Cursor 需要更丰富的 detected/configured/malformed/partial/drift/collision/effective 状态模型 |
| Tests | `src/install/tests.rs:14`、`src/install/tests.rs:152`、`src/doctor/environment.rs:685` | 有隔离 HOME、dry-run no-store 与 host probe fixture 模式 | 扩展为两文件故障矩阵与 secret/perms/concurrent edit 回归测试 |

## 设计方案

### 1. 依赖门与 canonical host

`SP824-T0` 的三个门全部通过后，GH-823 必须已经批准并提供：

- canonical `cursor` identity 与 closed-set parser；
- Cursor hook 命令的 event、subcommand、args 与输出合同。

GH-824 自己拥有 install-side `runtime_host_name("cursor")` mapping、`hosts.cursor`
defaults/normalization、`install_receipt` schema 及 cleanup/compatibility policy；这些都不
反向声明为 GH-823 的交付物。mapping 必须引用 GH-823 批准的 canonical identity，支持
检查针对当前 binary 的 closed-set parser 和 GH-824 install contract version，而不是
要求用户 config 已经有 `hosts.cursor`。首次安装缺少该 section 时，先完成两个 Cursor
文件的只读 preflight，再由 `ensure_config_for_hosts` 从 GH-824 批准的 defaults
materialize `hosts.cursor`；runtime config snapshot、materialization 与 install receipt
写入由同一 coordinator 纳入 rollback。任何 `unknown` 结果或不受支持的 contract
version 都是内部合同错误，整个 Cursor host 不写文件；单纯缺少用户 section 不是错误。
contract v1 defaults 精确为：

```toml
[memory_ai.hosts.cursor]
memory_profile = "codex"
context_gate = "strict"
context_color = true
capture_adapter = "cursor"
```

materialization 只补缺失字段并保留类型/闭集值合法的显式用户值；但
`capture_adapter` 对 `hosts.cursor` 只接受精确 `cursor`，任何其他字符串均在 plan
阶段失败，不能保留或路由到 Claude/Codex parser。其他错误类型/非法闭集值同样失败。
`context_gate` 不得沿用 generic host 的 `off` fallback。
本 PR 不在 Draft spec 中猜 GH-823 的最终 hook payload。

### 2. Cursor 专用 parser 与 managed ownership

新增 `src/install/hosts/cursor.rs`，但不要复用
`config.rs::remove_remem_hooks()`。用 typed document 表示两个文件：

- `CursorHooksDocument`: root object；`version` 必须为 JSON integer `1`；
  `hooks` 必须为 object。`CursorHooksV1Schema` 遍历所有 event，而不是只遍历
  managed event。冻结的 event 闭集是
  `beforeShellExecution`、`beforeMCPExecution`、`afterShellExecution`、
  `afterMCPExecution`、`beforeReadFile`、`afterFileEdit`、
  `beforeTabFileRead`、`afterTabFileEdit`、`stop`、
  `beforeSubmitPrompt`、`afterAgentResponse`、`afterAgentThought`、
  `sessionStart`、`sessionEnd`、`preCompact`、`subagentStart`、
  `subagentStop`、`preToolUse`、`postToolUse`、`postToolUseFailure`、
  `workspaceOpen`。entry 是 exact tagged union：
  command variant 为 optional `type:"command"` + required non-empty
  `command`；prompt variant 为 required `type:"prompt"` + required non-empty
  `prompt`，两者禁止携带另一 variant 的 payload field。共同 optional field 只有
  positive finite number `timeout` 与 boolean `failClosed`。non-empty string
  `matcher` 只允许在 tool 三事件、subagent start/stop、shell before/after、
  beforeReadFile、afterFileEdit、beforeSubmitPrompt、stop、
  afterAgentResponse、afterAgentThought；`loop_limit` 只允许在
  stop/subagentStop 且类型为 positive integer 或 null。未知 event、未知 entry
  shape、额外字段或错误类型一律 fail closed。通过 schema 但不匹配 remem ownership
  的 foreign entry 只保留、不改写、不归类为 managed 或 collision。schema 漂移由新
  contract version 承接，v1 parser 不放行猜测字段。
- `CursorMcpDocument`: root object；`mcpServers` 缺失时可在 plan 中创建，存在时
  必须是 object，并遍历校验每个 server value。foreign entry 是 exact untagged union：
  explicit stdio variant 为 required `type:"stdio"`、required non-empty `command`、
  optional string-array `args`、optional string-map `env`、optional non-empty
  `envFile`；documented-example compatibility variant 省略 `type`、required non-empty
  `command` 且 optional 字段相同，只用于 foreign validation/preservation；remote variant 为
  required non-empty `url`、optional string-map `headers`、optional
  `type:"http"|"sse"`、optional exact `auth` object。`auth` 要求 non-empty
  `CLIENT_ID`，可选 non-empty `CLIENT_SECRET` 与 non-empty-string array `scopes`，
  且无未知字段。两个 variant 不得混合 command/url 或另一 variant 的字段，
  且不允许未知字段；非 object entry、缺 transport identity、错误类型或未知 shape
  使整份文档在写前失败。contract v1 `remem` current shape 精确为
  `{"type":"stdio","command":"<canonical binary_path>","args":["mcp"]}`，只允许这三个
  字段，不含 `transport`/`env`。这是 Cursor 官方 detailed stdio table 的 required
  type/command 与 optional args 形状；使用 Cursor 专用 builder，不能直接复用可能
  携带其他 host 字段的 Claude builder。`remem` value 要么精确 current shape，
  要么精确命中 legacy allowlist，否则为 collision。

managed ownership 使用结构化 equality 与 versioned receipt，而不是命令 substring。
current shape 由一个 builder 产生，并由同一个 matcher 比较。GH-824 contract v1 的
remem-managed entry schema 只有 `command: string` 与
`timeout: positive integer seconds`；`type`、`prompt`、`matcher`、
`failClosed`、`loop_limit`、独立 `args` 和任何其他字段都不允许。builder 的 exact
component table 为：

| Component | Event(s) | Exact command after typed path substitution | Timeout | Gate |
| --- | --- | --- | ---: | --- |
| `observe_generic_success_v1` | `postToolUse` | `<binary_path> observe --host cursor` | 120 | only with the same total delivered-failure policy as the failure component |
| `observe_generic_failure_v1` | `postToolUseFailure` | `<binary_path> observe --host cursor` | 120 | only after GH-823 implements a total delivered-failure policy |
| `observe_mcp_specific_v1` | `afterMCPExecution` | `<binary_path> observe --host cursor` | 120 | only when the observe bundle is enabled and GH-823 B-016 has approved a stable opaque specific-event per-call ID, then selects after-only MCP ownership |
| `summarize_stop_v1` | `stop` | `<binary_path> summarize --host cursor` | 120 | only when GH-823 Stop wiring and GH-825 reader are runtime-proven |

observe components 是一个 atomic managed bundle：
`observe_generic_success_v1` 不得脱离 `observe_generic_failure_v1` 单独安装。
total policy 不能只列 observed failed Read。批准的 `postToolUseFailure` shapes 进入
bounded canonical capture；任意其他结构合法但未批准的 delivered failure tool name
必须 explicit success/no-op，zero capture/enqueue/spill/adapter/database write，并写
固定、无 raw payload 的 error-level unsupported diagnostic。该策略不改变成功
`postToolUse` 的 GH-823 generic contract：结构合法的未知 non-MCP tool name 仍 verbatim
capture；只有下述 MCP-specific ownership 分支会对已验证的 generic MCP success
zero-write。若 GH-823 没有覆盖 failure 的 capture/zero-write 两类分支或没有冻结
failure precedence，builder 完全省略所有 observe components；
不得留下 success-only capture，也不得全局注册后依赖偶然 parse error。

`observe_generic_success_v1` 在 MCP-specific ownership 下仍为 Read/Shell 等非 MCP 工具注册。
该分支在 #822/GH-823 批准 stable opaque specific-event per-call ID 前不可选；不得以
server/tool/generation 或 input/result/duration hash 充当 call identity。
由于 remem managed v1 entry 不生成 matcher，该模式必须由 GH-823 parser 对已验证
`tool_name: "MCP:<name>"` 的 generic delivery 执行 explicit success/no-op：zero canonical
capture、enqueue、spill、adapter/database write；只注册并解析 terminal
`afterMCPExecution`，`beforeMCPExecution` 不注册、不进入 capture。
generic ownership 不注册任何 specific event，也不执行该 generic-MCP drop。双投递 fixture
必须证明每个 MCP call 只有一个 canonical capture；同一 generation 两次相同 MCP tool 调用
必须得到不同 key，而各自 replay 必须命中原 key。

`sessionStart` and `preCompact` are absent from v1: the former is blocked for
Cursor 3.12.17 and the latter has no approved remem action. `timeout: 120` is
an install policy proposed and frozen by this packet's eventual human approval,
not a claim derived from the probe's diagnostic timeouts. `binary_path` is the
only field allowed to change across installations and is the receipt-bound
typed slot; it is not a wildcard. Contract version, host, component, event,
command tail and timeout live in the receipt metadata/canonical digest rather
than undeclared CLI flags.

Cursor 3.12.17 的 PR #914 evidence 证明默认 non-zero/timeout hook 后 host prompt 继续，
且 UI 未显示错误。虽然 foreign schema 允许 `failClosed`，managed builder 不生成它，
也不把未实测的 host behavior 当作 capture guarantee。实现必须把两个边界分开：

- remem-side fail-closed：malformed payload、未知 event 或 event/command mismatch
  non-zero，error-level log，stdout empty，zero
  capture/enqueue/spill/adapter/database writes；唯一例外是 total-policy 已覆盖的
  `postToolUseFailure` 中结构合法但未批准的 tool name，它 exit 0 explicit no-op，
  写固定无 payload 的 error-level diagnostic，并保持相同 zero-write 边界；
- host-side delivery：`host_continues` residual risk，不能宣称 prompt 被阻断或 missing
  capture 被 host 防止。

install/dry-run/doctor 的 human 与 JSON 输出都报告
`hook_failure_policy: host_continues`；`effective` 聚合不能把它提升成 host-blocking。

hook command rendering 复用当前 `src/install/config.rs::shell_quote` 的精确 POSIX
算法：非空且所有字符属于 ASCII alphanumeric 或 `/._-` 的 path 原样输出；否则用单引号
包裹，并把每个单引号替换为 `'\''`，再追加固定 tail
` observe --host cursor` 或 ` summarize --host cursor`。receipt 保存 canonical unquoted
`binary_path`、rendered command、tail 与整项 digest；ownership matcher 必须重建后精确比较，
不能用 substring。该 renderer 只获准用于 macOS/Linux；Windows/UNC 在真实 fixture 和人工
决策前为 unsupported。Auto 对该 host non-fatal skip，显式 Cursor/All 在任何 host 写入前
fail closed。

canonical runtime config 的 `hosts.cursor.install_receipt` 是非敏感、版本化 object：

- `schema_version`、`mode: full|hooks_only`、`contract_version`；
- 上次成功安装的 canonical `binary_path`；
- 每个 managed MCP/hook component 的 exact event/key、component ID、canonical unquoted
  binary path、rendered command 或 MCP type/argv、command tail、timeout 与 canonical JSON
  SHA-256 digest。

识别旧 binary path 条目时，matcher 必须同时验证：批准 event/key/component/command tail/
timeout、entry 中的 path 等于 receipt `binary_path`、整项 canonical digest 等于 receipt
记录。全部命中后才可把 path 更新为当前 builder，并在成功 apply 时原子更新 receipt。
component/command 单独命中、receipt 缺失/篡改、path 或 digest 不符全部 collision；不得退化为
basename、`contains("remem")`、path contains 或任意 subcommand。legacy allowlist 是
有限版本化 receipt/shape 枚举，无 wildcard。已证明的 legacy shape 在 install 时升级、
uninstall 时删除；无法证明 ownership 的同名/同 event entry 不动并 fail closed。

foreign preservation 以同一已验证 plan snapshot 删除 managed paths 前后的
`serde_json::Value` 投影比较：object key 顺序与 whitespace 不进入合同，array 顺序和所有
value 必须相等。B-012 final comparison 观察到的新 external version 直接 abort 并保留，不进入
旧 plan 的投影比较；post-comparison/pre-rename 才发生且被 rename 覆盖的不可观察编辑不在
B-008 保证内。测试同时覆盖 key 中含 `remem`、command 路径含 `remem`、未知 remem
subcommand，证明它们不会被 plan snapshot 的 managed cleanup 删除。

### 3. 统一 plan：所有命令先验证两个文件

定义无副作用的 `CursorConfigPlan::build(operation, mode, paths)`：

1. 读取两个 Cursor 路径及 canonical runtime config/receipt 的 raw bytes、metadata、
   存在性与 file identity。
2. 完整 parse/schema validate 两个文件；hooks 校验覆盖整个 version-1 event/entry
   tree，而不只是 managed 子集；即使 `hooks-only` 也校验 MCP 文件。
3. 检查 current managed shape、receipt-bound old-path exact ownership、legacy allowlist、
   collision 与 partial/drift/intentional hooks-only 状态。
4. 生成两个 `FilePlan` 和一个 `RuntimeConfigPlan`：`add/remove/replace/no-op/would-fail`、
   新 bytes、原 snapshot、是否允许 apply；首次安装的 `hosts.cursor` materialization 与
   最终 receipt 都只存在于 plan 中，不能在 Cursor preflight 前落盘。
5. 输出只含路径、action 与固定错误 code；不包含 raw JSON、env 或 command 中的
   secret-bearing value。

orchestrator 必须先为所有 selected host 构造无副作用 preflight，且 Cursor plan 必须在
`ensure_runtime_store_ready()`、`ensure_config_for_hosts()`、API token、任何 host
`install_mcp/install_hooks` 或 data-dir 创建之前成功。显式 Cursor/All 的
unsupported renderer、任一 Cursor schema/collision/receipt 错误，以及 Auto 的
unsupported-Cursor skip 都在此阶段决定。只有所有 preflight 成功后才可初始化既有
runtime store 流程并 apply；因此首次运行的 Cursor preflight failure 对
store/key/database/token/runtime config 和所有 host 配置均为零副作用。后续 apply/write
故障继续走 B-010 compensating rollback，不把 runtime store 初始化误称为跨文件事务的一部分。

`install`、`uninstall`、`hooks-only`、`dry-run` 与 doctor 共用 parser 与状态分类。
dry-run 只 render plan，绝不调用 runtime-store/token 初始化或 temp writer。
`hooks-only` 的 MCP `FilePlan` 固定为 validated/no-change，但另一文件失败仍使整个
命令 would-fail；成功 plan 的 `RuntimeConfigPlan` 记录 `mode: hooks_only`。full install
记录 `mode: full`，uninstall 清除 receipt。receipt 失败与 Cursor 文件失败使用同一
compensating rollback，避免 UI 宣称安全而 durable intent 不一致。

### 4. 安全单文件 staged writer

不要直接调用当前 `atomic_file::write_atomic` 处理 Cursor secret-bearing JSON，
因为它在新 temp 写入后才复制权限。增加可复用的 secure staged writer（最终文件
位置在实现阶段由 maintainer 确认），满足：

1. 在目标同目录以 `create_new` 创建不可预测的 fresh temp；禁止复用旧 temp。
2. 在任何 `write_all` 之前，无论目标是否存在，Unix 都必须把 temp 设为 `0600`
   并移除继承/default ACL；其他平台必须使用可证明等价的 owner-only primitive，
   否则 fail closed。不得把现有目标可能宽松的 mode/ACL 复制到 temp，也不得先写
   内容再 chmod。replace 后目标保持 owner-only，不恢复宽松权限。
3. 写完整 bytes、`sync_all`、关闭 handle；重新核对目标 snapshot 后才 replace。
4. replace 后 sync parent directory（平台支持时）。
5. 所有失败路径 best-effort 删除 temp；cleanup 失败附加到主 error，不打印内容。

为 writer 增加 permission-before-write、write/sync/rename/dir-sync/cleanup failpoint；
测试观察 temp mode 与残留文件，而不是弱化现有 atomic writer 测试。

### 5. 两文件与 runtime receipt 的 staged apply / 补偿回滚

Cursor host 使用一个协调入口，替代 trait 方法被 runtime 顺序调用的行为。可选择
给 `InstallHost` 增加 `apply_install_plan`/`apply_uninstall_plan`，或 runtime 对 Cursor
调用 transaction object；关键合同是一次 plan 统筹两个 Cursor 文件与 canonical
runtime config/receipt。

apply 步骤：

1. plan 阶段完成后，再核对三个 snapshot，任一变化立即 abort。
2. 为会变化的 Cursor 文件与 runtime config 准备 secure temp 并 fsync，但尚不
   replace；runtime config temp 包含 materialized host 与最终 receipt。
3. 按固定顺序处理会变化的 Cursor 文件；每次 rename 紧邻执行前用 raw bytes
   digest + 存在性做最终比较。若此时不等于 snapshot，立即 abort 并保留观察到的
   外部版本；否则 replace 并记录 committed path。
4. Cursor 文件都成功后，对 runtime config/receipt 做同样的最终比较再 replace；
   receipt 不能先于它所证明的 managed entries 生效。
5. 每次 replace 后 read-back planned bytes；不匹配说明已发生可观察 drift，停止
   后续 apply 并进入失败/rollback 路径。
6. 任一后续 replace 或 read-back 失败时，逆序处理已经提交的目标。restore 前最终
   比较当前内容是否仍等于本 transaction 写入的 planned bytes；不相等时保留该
   外部版本并报告 `partial_state`，不得用旧 snapshot restore。原先不存在的目标也
   只有在同一比较成立时才能删除，避免删除可观察到的并发创建/编辑。
7. rollback 后逐路径 read-back，验证 bytes/不存在性与 snapshot 相同。
8. rollback 或 read-back 验证失败时返回包含 `partial_state`、路径、receipt 状态与
   doctor repair action 的复合 error；永不只保留最后一个目标的原始 error。

这不是跨文件 atomic transaction。正常/失败输出、README 与 release note 都只能
称为 staged apply + compensating rollback。

concurrent edit identity 至少包含 raw bytes digest 与存在性；metadata 可辅助但不能
单独作为 identity。B-012 的保留保证止于每次 replace/restore 紧邻前的最终比较：
该比较看到 mismatch 时必须 abort 且保留外部 bytes。可移植的 compare + rename
不是 CAS；非协作进程可在比较返回相等后、rename 前写入，而本次 rename 随后可能
覆盖该编辑。若外部写发生在 rename 后、read-back 前，read-back mismatch 可检测并
触发失败/`partial_state`；若 post-compare 编辑已被 rename 覆盖且 read-back 只看到
planned bytes，则该覆盖不可由此机制检测，命令甚至可能按计划成功。实现、测试、
doctor、README 与 release note 都必须保留这一 residual risk，不能声称所有
post-preflight 编辑都受保护，也不能把 advisory lock 当作其他进程遵守的互斥或
无证据地称为 CAS。该窗口是明确的 residual user-data-loss risk，而不是可由
read-back、doctor 或 rollback 补偿消除的 `partial_state` 子类。

### 6. 具体 target 与输出行为

- `InstallTarget::Cursor` 与 `All` 显式选择 Cursor；如果当前平台没有获准 renderer，
  在任何 host 写入前 fail closed。`Auto` 以 `~/.cursor/` directory 或已有任一用户级
  Cursor config 为 detected，但只在 renderer-supported macOS/Linux 上把 Cursor 加入
  plan；unsupported Windows/UNC 跳过 Cursor、输出稳定的 non-fatal diagnostic，并继续
  Claude/Codex。该 skip 不得创建 Cursor path/config/receipt，也不得把 Cursor 报成
  configured。
- Cursor `dry_run_plan` 不再只是字符串列表，而从 typed plan render 两个绝对路径
  及 action。uninstall dry-run 同样显示 hooks 与 MCP，而不是只有 primary path。
- `hooks-only` 仍先验证两文件，不写 MCP；real install 只在 runtime config、Cursor
  两文件 transaction 与后续步骤的失败语义明确后报告成功。
- multi-host 操作逐 host 记录 applied/failed/partial；Cursor 失败不得被后续 host
  输出覆盖。是否回滚其他已成功 host 不在 #824 范围，但最终退出必须非零且列明状态。

### 7. Doctor 状态模型

把 Cursor probe 从简单 `HostProbe` 扩展为结构化结果：

- `detected`: Cursor app/config directory 是否存在；
- `configured`: 文件 shape 与 install receipt 对声明 mode 一致；
- `configured_mode`: `full|hooks_only|none|unknown`；只有 receipt 的 mode、binary path、
  component digests 与实际 entries 全部匹配时才报告 `full`/`hooks_only`；
- `malformed`: JSON parse/root/version/container/event/type 错误，带路径和 code；
- `partial_state`: 文件与 receipt mode/digest 不一致，或 rollback evidence/read-back
  显示未恢复；匹配 receipt 的 intentional `hooks_only` 明确不是 partial；
- `drift`: managed key 存在但不等于 current/legacy exact shape；
- `collision`: 用户条目占用 `mcpServers.remem`，或 managed event 中有疑似但不精确
  ownership；
- `effective`: 汇总 `proven/blocked/unknown`，并附 GH-822 evidence version 和逐 capability
  状态。PR #914 对 Cursor 3.12.17 的基线是
  `postToolUse_delivery: proven`、`postToolUse_managed_context: not_configured`、
  `sessionStart: blocked`、`stop/preCompact: unknown`；observe entry 只做 capture，
  不能因 host delivery proven 就冒充已安装 context producer 或把 aggregate 提升为
  proven；没有 capability-matched real-agent marker 与 managed producer 时绝不由 static
  parse 推导为 proven；
- `session_init`: 固定为 `unsupported`，并在 human-readable 输出显示精确行
  `session-init: not supported on cursor`。这是 GH-823 B-006 的 per-command capability，
  与 detected/configured/mode/effective 正交，在任何 Cursor doctor 状态下都不得省略。

doctor repair 先重新运行同一 parser，展示将 add/remove/replace 的路径与非敏感
reason；malformed/collision/concurrent state 不自动覆盖。intentional `hooks_only` 不
建议补 MCP 或清理 hook；receipt 缺失/不匹配的 ambiguous single-file state 才进入
partial-state repair。partial-state 可在用户确认后的 implementation command 中重新
apply 或 uninstall，但 doctor 本身保持 read-only。

### 8. 官方事实与 PoC evidence 边界

实现可直接编码的 2026-07-23 官方文档 snapshot 包括：`version: 1`、B-003 的
21-event lower-camel 闭集、command/prompt tagged entry、共同 timeout/failClosed、
event-specific matcher/loop_limit、用户/项目路径、自动 reload（失败时 restart）、
user/project command cwd、Enterprise→Team→Project→User 优先级、用户 hook 不进入
cloud VM，以及 MCP 用户路径/root key。

GH-822 evidence 独立存放，不把 raw/private payload 放入本 spec。PR #914 exact head
`c0802c42c3fc22770aecb0b7b2eec88f117f795c` 提供 Cursor 3.12.17 的 sanitized
fixture、隔离 workspace 与 real-agent marker 结果；只有该类 evidence 经人工采用才可决定：

- 哪些候选 event 进入 managed builder；
- exact managed event/command/timeout shape（remem managed v1 entry 明确无
  type/prompt/matcher/failClosed/loop_limit/独立 args；这些 foreign schema fields
  不等于 managed builder capability）；
- reload 后 event 是否生效，以及每个 context capability 是否对模型 effective；
- foreground/background/IDE/CLI 支持矩阵；
- context size 与 multi-root policy。

若 PoC 与 docs 冲突，记录版本化 drift，默认 fail closed，并由人工更新 GH-823/
GH-824；不得偷偷用“最接近”的 event 或 `hosts.unknown`。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| B-001 target/path semantics | `src/install/host.rs`、`hosts/mod.rs`、`paths.rs` | target matrix test: Cursor/Auto/All/Claude/Codex × missing/existing `.cursor` × macOS/Linux/unsupported Windows；Auto skips Cursor non-fatally on unsupported renderer and continues other hosts with zero Cursor side effects, while explicit Cursor/All fails before any host write |
| B-002 canonical host, no unknown | GH-823 identity + GH-824 runtime mapping/config | missing section materializes exact codex/strict/true/cursor defaults and no `hosts.unknown`; valid explicit non-adapter values are preserved; capture_adapter accepts only cursor and rejects claude-code/codex-cli/unknown/arbitrary strings before writes; wrong types/illegal values fail; runtime config failure rolls back Cursor files |
| B-003 strict whole-document hooks schema | `CursorHooksV1Schema` + typed parser | table tests iterate all 21 frozen event names, command/prompt discriminators, event-specific matcher allowance, stop/subagentStop loop_limit, common timeout/failClosed, plus non-object root, missing/wrong/non-1 version, unknown event/entry shape, non-array event, non-object entry, missing/extra/wrong-type fields; structurally valid foreign entries are preserved and every rejection leaves both raw files unchanged |
| B-004 strict MCP schema | Cursor typed parser + Cursor MCP builder | exact remem `{type:"stdio",command,args:["mcp"]}` snapshot passes and remem extra/missing/wrong fields collide; table tests accept frozen foreign explicit stdio and documented-example type-omitted stdio command/args/env/envFile variants plus remote url/headers/optional http-or-sse type/auth variants, including auth CLIENT_ID/CLIENT_SECRET/scopes types; reject array/null server values, no command-or-url identity, mixed command/url transport, unknown/extra/wrong-type fields, non-object root and array/null `mcpServers`, and leave both raw files unchanged on every rejection; missing container plans create |
| B-005 GH-823 event gate | managed builder + GH-822 fixture gate + shared POSIX quoting | PR #914 events enter only after adoption; safe/space/single-quote paths quote correctly; the entire observe bundle is absent until GH-823 total failure/precedence policy covers approved failure capture plus explicit zero-write/error diagnostic for every other delivered failure tool; successful unknown non-MCP postToolUse remains verbatim generic capture; generic ownership omits specific events and is the only selectable branch until evidence approves a stable opaque specific-event per-call ID; an approved specific branch registers only afterMCPExecution, keeps generic non-MCP capture while generic MCP is zero-write, proves two same-tool calls in one generation have distinct keys with per-call replay stability, and dual delivery produces one canonical capture |
| B-006 exact MCP ownership | MCP matcher + versioned install receipt | current exact type/command/args match and recorded old binary path + exact type/path/argv/digest upgrade; missing/tampered receipt, path/digest/field mismatch collide; foreign `remem-helper` remains untouched |
| B-007 exact hook ownership | hook matcher + versioned install receipt | each recorded old-path hook upgrades/removes only with exact event/component/rendered-command/tail/timeout/unquoted-path/digest; hook entry has no marker/matcher/args; commands containing `remem`, alternate basename/path/unknown subcommand remain; ambiguous entry collides |
| B-008 foreign semantic preservation | plan projection comparator | 对同一 validated snapshot 做 install/uninstall round-trip，比较 nested values 与 array order；final comparison 观察到新版本则保留并 abort；post-comparison residual 窗口只由 B-012 测试覆盖；formatting rewrite 明确允许 |
| B-009 validate all before writes | `CursorConfigPlan::build` + install orchestrator preflight phase | malformed hooks + valid MCP, inverse, malformed runtime config, collision and unsupported explicit renderer all fail before `ensure_runtime_store_ready`, runtime host config, token or any host apply; isolated first-run HOME proves no data dir/key/db/token/config/temp/mtime change; Auto unsupported skip continues supported hosts only after zero-side-effect Cursor preflight; error contains path/code only |
| B-010 staged apply + rollback | Cursor transaction coordinator | fail each Cursor replace and runtime receipt replace; all prior targets restore/read-back to snapshots; receipt never claims unapplied entries; output says `compensating rollback`, never cross-file atomic |
| B-011 rollback failure visible | transaction failpoints + error type | inject a later replace + restore failure; non-zero result includes `partial_state`, every target status and doctor action |
| B-012 concurrent edit boundary | snapshot final-comparison + replace/read-back checks | 对每个 Cursor target 与 runtime config/receipt 分阶段注入：(a) final comparison 前 mutate，断言非零 abort 且外部 bytes 保留；(b) comparison 返回相等后、rename 前由非协作 writer mutate，确定性展示该版本可被 planned rename 覆盖且不把结果宣称为受保护/可检测；(c) rename 后、read-back 前 mutate，断言 read-back drift 非零、路径/`partial_state`/doctor action 明确；rollback restore 使用相同三阶段矩阵 |
| B-013 secret-safe temp | secure staged writer | Unix tests start from missing, `0600`, permissive `0644`, and inherited-ACL targets and inspect fresh temp before the first byte: always `0600`, no inherited ACL; unsupported platform fails closed; sync/rename/cleanup failures leave no readable temp and logs omit sentinel secret |
| B-014 idempotency/no file deletion | plan/apply + receipt | install twice parsed/receipt equality; binary path change performs one exact receipt-backed update then converges; uninstall twice no-op; user-created empty files remain |
| B-015 shared validator all modes | command dispatch + typed plan + receipt | same malformed/collision fixtures produce same codes in all modes; hooks-only MCP validated/no-change and matching receipt yields configured_mode hooks_only; receipt write failure rolls back |
| B-016 dry-run zero side effects | plan renderer/runtime early path | subprocess with isolated HOME asserts two paths/actions, no secret, no dirs/temp/config/key/db/token, metadata unchanged |
| B-017 strict uninstall | transaction operation enum | malformed/collision/other-file failure prevents fuzzy removal; only exact current/legacy entries removed |
| B-018 doctor state dimensions | `src/doctor/environment.rs` or Cursor module | matrix: full receipt, intentional hooks-only receipt, no entries, hook-only without/mismatched receipt, MCP-only, digest/path/mode drift; assert configured_mode/partial plus other JSON fields, `session_init: unsupported`, exact `session-init: not supported on cursor` human line, and repair text |
| B-019 effective evidence gate | doctor + GH-822 evidence loader | PR #914 yields postToolUse delivery proven but managed context not_configured, sessionStart blocked, stop/preCompact unknown, and hook_failure_policy host_continues; observe never counts as a context producer; remem-side zero-write is not host blocking; aggregate never proven while required capability blocked/unknown/not_configured; stale evidence cannot promote |
| B-020 uninstall-before-downgrade | uninstall output/docs/doctor action | golden output orders current-version uninstall before downgrade; legacy-shape fixture gives repair instruction |
| B-021 docs-vs-PoC separation | evidence schema/spec tests | check official facts, PR #914 observed facts and remaining unknowns are separate; one capability/version cannot promote another and unknown candidates cannot enter managed builder |
| B-022 other hosts unchanged | existing install/doctor modules | focused Claude/Codex regression tests plus full `cargo test`; multi-host Cursor failure exits non-zero with per-host status |

## 数据流

```text
CLI target/mode
  -> resolve detected Cursor + canonical GH-823 host gate
  -> read both raw files + snapshots
  -> strict parse/schema/ownership classification
  -> immutable two-file + runtime config/receipt plan
       -> dry-run: redact + render only
       -> doctor: classify only
       -> real apply: secure stage changing Cursor files + runtime config
            -> compare snapshots
            -> final compare file A (preservation guarantee ends here) -> replace -> read-back
            -> final compare file B -> replace -> read-back
            -> final compare canonical hosts.cursor + final receipt -> replace -> read-back
            -> on failure: reverse compensating restore + verify
                 -> restored | explicit partial_state
```

持久化输出是两个用户级 JSON 文件及 canonical runtime `hosts.cursor` config/receipt；
该 section、defaults/normalization、receipt schema 与 cleanup/compatibility policy 均由
GH-824 定义，GH-823 只提供 canonical host/hook 合同；receipt 只存非敏感
mode/path/digest。
日志与 doctor 只接收 stable code、path、action、state，不接收 raw JSON。

## 备选方案

- 复用 `read_json_file`/`write_json_file`：拒绝。它会 pretty-print 全文且当前 temp
  权限设置晚于 secret 写入；也没有两文件协调或 ownership typed validation。
- lossless JSON token editor：暂不采用。产品只在 B-008/B-012 的 snapshot/final-comparison
  可观察边界内要求 foreign 语义保留，不要求 whitespace/key-order byte preservation；
  token editor 增加 parser 与安全复杂度而没有用户价值。
- 用 `contains("remem")` 清理：拒绝。无法证明 ownership，会删除用户数据。
- 宣称真正跨文件 atomic：拒绝。普通文件系统的两个 rename 没有共同 transaction；
  staged apply + 可验证 compensating rollback 是诚实边界。
- 把 advisory lock 或 compare-before-rename 称为 CAS：拒绝。Cursor/编辑器等非协作
  writer 不保证遵守 lock，portable compare 与 rename 之间仍有可覆盖外部编辑的窗口；
  没有已证明的 no-clobber primitive 时只能收窄保证并显式测试 residual risk。
- 先降级再由旧版本卸载：拒绝。旧版本不知道新 shape，可能遗漏或误删。

## 风险

- Security: foreign MCP env 可能含 secret；通过 permission-before-write、日志 redaction、
  temp cleanup 与 failpoints 防止短暂泄漏。
- Compatibility: Cursor schema/event 会演进；strict version/shape + versioned legacy
  allowlist + GH-822 evidence gate 防止静默漂移。
- Concurrency: final compare 只保护比较时已经可观察到的编辑；compare→rename
  之间的非协作写可能被覆盖且在 read-back 只看到 planned bytes 时不可检测。
  post-rename drift 由 read-back/`partial_state` 暴露；文档和测试不得把 advisory
  lock、compare-before-write 或 doctor 夸大为 CAS/全时段保护。前者是 residual
  user-data-loss risk，不能被归入“所有 foreign data 都保留”或“所有并发编辑都
  fail closed”的产品承诺。
- Recovery: rollback 自身可能失败；复合 error 和 doctor repair path 必须保留两个
  failure，不允许 warning + success。
- Maintenance: product-to-test mapping 较大；task plan 按 parser/ownership、secure
  writer/transaction、dry-run/doctor、verification 四个可审查阶段记录依赖。该计划
  可以在 Draft 阶段评审，但所有实现阶段都受 `SP824-T0` 约束。

## 测试计划

- [ ] Unit: whole-document typed parser（21-event command/prompt、matcher/
      loop_limit/failClosed 与 version matrix）、exact
      ownership/legacy allowlist、Cursor MCP type/command/args exact shape、
      safe/space/single-quote POSIX path rendering、Auto Windows/UNC skip 与 explicit
      target fail-closed、observe-bundle absent/total-policy 两分支、
      generic-vs-specific MCP after-only single-capture、
      capture_adapter exact-cursor rejection、foreign projection、plan renderer、
      `hook_failure_policy: host_continues` doctor classifier。
- [ ] Failure injection: missing/permissive/ACL target 的 temp owner-only-before-first-byte、
      write/sync/rename/cleanup、每个后续目标 replace、rollback/rollback verify、
      concurrent edit 三阶段矩阵（final comparison 前保留、post-comparison/pre-rename
      residual overwrite、post-rename/read-back drift）。
- [ ] Integration: isolated HOME 的 install/uninstall/hooks-only/dry-run/Auto/All，
      canonical runtime config，以及 Cursor preflight failure 发生在 store/key/db/token/
      runtime config/任一 host write 前的 zero-side-effect assertions。
- [ ] Regression: Claude/Codex install/doctor tests 不降低断言；完整 Rust suite。
- [ ] Real host: 固定 PR #914 exact-head Cursor 3.12.17 sanitized fixture，验证 event
      invocation、热 reload、foreground payload、postToolUse proven/sessionStart blocked
      marker 结论；另行补 context size、background/CLI、multi-root、Windows 和
      stop/preCompact outcome。未知项维持 blocked/unknown，不修改测试使其“通过”。

实现 PR 的 fresh commands：

```bash
cargo fmt --check
cargo check
cargo test install
cargo test doctor
cargo test
python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md
```

本 spec-only PR 的 fresh checks：

```bash
PYTHONPATH=checks python3 -c 'from pathlib import Path; from sensitive_enforcement import parse_planned_changes_manifest; m=parse_planned_changes_manifest(Path("specs/GH824/tech.md").read_bytes()); assert m["version"] == 1 and m["issue"] == 824 and m["complete"] is True'
python3 checks/check_workflow.py --repo .
python3 checks/check_workflow.py --repo . --spec-dir=specs/GH823
python3 checks/check_workflow.py --repo . --spec-dir=specs/GH824
python3 checks/github_issue_evidence.py --github-repo majiayu000/remem \
  --issue 824 --json > /tmp/gh824-issue-evidence.json
python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem \
  --issue 824 --json > /tmp/gh824-duplicate-evidence.json
python3 checks/route_gate.py --repo . --route write_spec --issue 824 \
  --evidence /tmp/gh824-issue-evidence.json \
  --duplicate-evidence /tmp/gh824-duplicate-evidence.json \
  --artifact product_spec=specs/GH824/product.md \
  --artifact tech_spec=specs/GH824/tech.md \
  --artifact task_plan=specs/GH824/tasks.md --json
git diff --check
python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md
```

GH-823 的 packet 内容由当前叠加 head 做结构检查，但其独立 exact-head
`write_spec`/CI 证据必须取自 GH-823 spec PR；GH-823 状态推进后不得在 GH-824 上重跑
`write_spec` 冒充历史 gate。packet check 只验证三份规划文档完整且相互一致，不代表 spec approval，也不把当前
`ready_to_spec` 转为 `ready_to_implement`。实现前仍必须用当前 GH-822/GH-823/GH-824
trusted GitHub evidence 与 GH-824 duplicate-work evidence 重新运行 implementation route
gate，并取得人工门通过的 `allowed` 结果；调用方自报 `--state`/`--label` 不能替代这些
evidence。

## 回滚方案

发布后的功能回滚顺序固定为：

1. 用当前版本运行 `remem uninstall --target cursor`，通过 current builder 或
   exact builder/receipt metadata + digest 识别并移除 current/recorded legacy managed entries，同时
   清除 managed install receipt。
2. 运行 `remem doctor`，确认两文件无 `partial_state`、collision 或 drift；foreign
   semantic projection 保持。
3. 再降级或回滚二进制/实现 PR。

若 uninstall 或 compensating rollback 失败，不继续降级。保留当前二进制和 snapshot
evidence，doctor 显示具体路径与 repair action，由用户处理 malformed/collision 或
重试安全恢复。无数据库 migration；`hosts.cursor` defaults/normalization、receipt 与
清理/兼容策略由 GH-824 合同决定，绝不回落到 `hosts.unknown`。
