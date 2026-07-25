# GH761 Tech Spec: Claude hook integrity self-check and repair mode

Issue: https://github.com/majiayu000/remem/issues/761
Product spec: `specs/GH761/product.md`
Status: Draft for SpecRail approval (2026-07-08)
Base: origin/main（写作时 694d4e6）

## 1. CLI 面

`src/cli/types.rs` 的 `Install` 命令新增：

```text
remem install [--target <auto|all|claude|codex>] [--hooks-only] [--repair] [--dry-run]
```

首版 repair 语义：

- `remem install --target claude --repair`：只执行 Claude hook repair，不安装 MCP，不清理 settings 中的 legacy MCP entry，不初始化 runtime store，不写 API token。
- `--dry-run --repair`：打印将检查/修复 `~/.claude/settings.json`，不写磁盘。
- `--target auto --repair`：仅在检测到 Claude 配置时 repair Claude；未检测到 Claude 时失败并提示使用 `--target claude` 强制目标。
- `--target all --repair`：可以遍历支持 repair 的 host，但本 issue 只要求 Claude repair；Codex 目标首版应返回明确 unsupported 或跳过且说明原因，不能伪装成功。
- `--repair` 与 `--hooks-only` 同时出现时 repair 语义优先；两者都不写入或清理 MCP。Repair 可以只读 `.claude.json` MCP command，用于输出 stale-MCP/install-path 诊断。

## 2. Hook integrity 模块

新增或抽出一个可复用模块，例如 `src/install/hook_integrity.rs`，负责读侧 hook 完整性评估。实现不得复制 doctor 解析逻辑。

推荐结构：

```rust
pub(crate) struct HookIntegrityReport {
    pub host: &'static str,
    pub expected: usize,
    pub registered: usize,
    pub path: PathBuf,
    pub missing_events: Vec<&'static str>,
    pub status: HookIntegrityStatus,
}
```

- 复用 `src/doctor/hook_validation.rs` 中的 expectation、command parsing、host normalization。
- 如现有 `pub(super)` 可见性不足，应把 hook expectation/parse helpers 移到共享模块，doctor 与 runtime self-check 都调用同一实现。
- Claude expected events 继续是 5 个：`PostToolUse`、`PreCompact`、`Stop`、`SessionStart`、`UserPromptSubmit`。
- 完整性判断必须复用 doctor 的 expected executable 选择：优先使用 Claude MCP remem command，其次使用 hook 文件中可唯一识别的 matching remem path，最后才使用当前进程 path。不能只按当前进程 path 校验，否则 stale hook binary 自检会漏报。
- 完整性判断必须校验 remem-managed matcher：`SessionStart` 期望 `startup|resume|clear|compact`，`PostToolUse` 期望 `Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task`，其余 expected entries 没有 matcher。命令存在但 matcher 被缩窄或丢失时不能报告 5/5。
- 完整性判断必须校验 remem-managed timeout：`context` / `session-init` 期望 15，`observe` / `summarize` 期望 120。Claude hook `timeout` 是秒单位；实现不得把 15000/120000 毫秒值写回 Claude settings。Timeout 缺失或偏离当前 expected seconds 时不能报告 5/5。
- 完整性判断不能只证明 expected entries 存在；同一 Claude event 中存在 parser 识别的额外 remem-owned hook（旧 binary、旧 matcher、旧 timeout、旧 hostless 形式等）时也必须 unhealthy，并给出 stale/duplicate detail。
- Hook removal 必须使用同一 parser 判断 remem-owned invocation，覆盖 shell form (`command` 字符串) 与 exec form (`command` + `args`)：executable file stem 是 `remem`，subcommand 是该 event 的 expected subcommand，host 是 `claude-code`、legacy env host 可归一到 `claude-code`，或 hostless legacy remem invocation。不能用 `command.contains("remem")` 或 path substring 删除 entries。Hostless legacy entries 只用于 repair removal/convergence；完整性 evaluator 仍应要求新版 host-aware hooks。

## 3. Runtime self-check

在 Claude `SessionStart` context 路径增加只读自检：

- 入口：`src/context/render.rs` 中 `generate_context_for_invocation`。自检应在 DB open 之前或以 helper 形式同时覆盖 DB-open failure early-return 分支和正常 `ContextGateDecision.output` 分支。
- 触发条件：source 是已安装 Claude SessionStart matcher 会触发的事件（startup/resume/clear/compact）或 Claude 传入的空 source，并且 active invocation 有 Claude 证据：`invocation.host == HostKind::ClaudeCode`，或 hook payload / env / CLI 显式归一到 `claude-code`。不要只凭全局 `~/.claude/settings.json` 中存在 hostless legacy hook 就在 `HostKind::CodexCli` invocation 上追加 warning；这会污染 Codex SessionStart JSON。Hostless legacy hooks 由 repair convergence 处理，runtime warning 仅在能证明当前 invocation 来自 Claude 时输出。
- 输出位置：context header 后或 context body 前，追加短 warning block，例如：

```text
## Hook Integrity Warning
- Hooks (claude) stale or incomplete: 3/5 registered in ~/.claude/settings.json.
- Repair: remem install --target claude --repair
```

- 对 Codex `SessionStart` JSON wrapper 不追加人读 warning，避免破坏 Codex hook JSON contract。
- 自检读取/解析失败时输出 warning，保留原 context 内容；不得因为自检失败返回空 context。
- 自检 OK 时不改变输出。
- Context gate 已经返回 empty/suppressed output 时，如果 integrity unhealthy，仍必须输出 warning。实现可在 gate decision 后追加 warning，或让 unhealthy integrity 强制 gate 发出 warning-only output。
- DB/key/schema 错误导致 context load 失败时，如果 hook integrity unhealthy，DB error output 与 hook warning 都必须出现；该 self-check 不依赖数据库连接。

## 4. Repair 实现

在 install runtime 增加 repair path，避免复用完整 install 的 runtime store 初始化副作用。

推荐实现：

```rust
pub fn install(target: InstallTarget, dry_run: bool, hooks_only: bool, repair: bool) -> Result<()>
```

当 `repair == true`：

1. 解析目标 host。
2. 对 Claude 调用专用 hook-only repair 函数，例如 `ClaudeHost.repair_hooks_only(&bin)`。不能调用现有 `ClaudeHost.install_hooks(&bin)`，因为该方法会清理 settings 中的 legacy MCP entry，违反 repair 不触碰 MCP 的副作用边界。
3. 该函数可以复用 `build_hooks(bin, HookStrategy::ClaudeCode)` 和 JSON read/write helpers，但不能复用当前 substring-based `remove_remem_hooks`。必须新增 parser-based removal，只移除当前 host/event 下可识别的 remem-owned expected hook inner commands，包括 hostless legacy remem invocations；如果一个 Claude hook object 的 `hooks` array 同时含 remem 和第三方 commands，必须只移除 matching remem inner commands 或拆分对象，保留第三方 sibling commands 与 matcher 字段。
4. 写入后立即做 hook JSON convergence check：用户级 `~/.claude/settings.json` 中 5 个新版 remem hook entries、matcher、timeout、subcommand、host 和当前 repair binary 均匹配，且不存在额外 parser-recognized Claude remem hooks，即为 repair 成功。这个 check 不使用 stale MCP path 作为硬失败条件。
5. 输出明确结果：`hooks -> ~/.claude/settings.json (5/5 registered)`；repair 可以只读 `.claude.json` MCP command。若 doctor-style evaluator 会因 stale MCP 继续把 expected executable 绑定到旧 binary，repair 仍可成功，但输出必须追加 warning/detail，提示 stale MCP/install-path 仍需完整 `remem install --target claude` 或 doctor 修复，而不是声称整机健康。

Repair path 不应：

- 调用 `ensure_runtime_store_ready`。
- 写或清理 `.claude.json` MCP。
- 清理 `~/.claude/settings.json` 中历史误写的 `mcpServers.remem` entry。
- 创建 API token。
- 删除第三方 hook entries。
- 写 project/local/managed-policy/plugin hook locations。若检测到这些 locations 存在 remem hooks，repair 输出必须提示它们未被本命令修复，用户可用 `/hooks` 或后续 doctor 扩展检查。

## 5. Doctor 兼容

`src/doctor/environment.rs` 可继续负责 doctor report，但 hook count 应来自共享 evaluator 或共享 expectation helpers。

修复后 doctor 的 Claude hook check 在 MCP 缺失或 MCP 指向当前 binary 时必须继续输出 `5/5 registered in <path>`。如果 MCP 指向 stale binary，doctor 现有 drift 检测不能被削弱：hook check 可以继续按 MCP 期望 executable 报告不匹配，或新增清晰 detail 说明 hooks 已 repair 但 MCP/install path stale。部分缺失时现有 `Warn` 语义保留，但详情里的修复命令应升级为 `remem install --target claude --repair`。

## 6. 文档

更新 README 的 install/doctor/troubleshooting 片段：

- 说明 `remem install --target claude --repair` 用于恢复丢失 hook。
- 说明 SessionStart warning 的含义。
- 明确 repair 保留第三方 hook，不写 MCP/runtime store；只读 MCP 检查仅用于 stale-path 诊断；首版 repair 只覆盖 user-level Claude settings。

若 CLI help 文本包含 install flags，也要同步。

## 7. 测试计划

| 测试 | 类型 | 断言 |
|---|---|---|
| shared evaluator detects 3/5 | 单元 | 删除 `PostToolUse` 与 `Stop` 后 registered=3，missing 包含两个事件 |
| configured executable drift | 单元 | MCP 指向 current binary 但 hook entries 指向 stale binary 时 self-check/doctor 都报告 stale/incomplete，而不是按当前进程误判 5/5 |
| matcher drift | 单元 | 五个命令存在但 SessionStart/PostToolUse matcher 被缩窄或丢失时 evaluator 不报告 5/5，repair 后 matcher 收敛 |
| timeout drift | 单元 | 五个命令和 matcher 存在但 remem timeout 被缩短、丢失或写成毫秒值时 evaluator 不报告 5/5，repair 后 timeout 收敛到秒单位 |
| exec-form hook parsing | 单元 | `{"command": "/old/remem", "args": ["observe", "--host", "claude-code"]}` 被识别为 removable/stale remem hook |
| extra stale remem hooks | 单元 | Fresh 5/5 hook set 外另有 stale remem Claude hook 时 evaluator unhealthy，repair 后只剩 expected set |
| user-level repair scope | 单元 | project/local Claude hook location 存在 remem hooks 时，repair 不写该文件并输出 scope warning |
| context warning visible | 单元/集成 | Claude SessionStart 输出含 warning、3/5、repair 命令；context gate suppresses normal output 和 DB-open failure 仍显示 warning；Codex JSON 输出不被污染 |
| repair restores hooks | 单元/集成 | repair 后 Claude settings 中 5 个 remem hook 存在 |
| repair preserves third-party hooks | 单元 | 非 remem entries 保留，包括 command/path 含 `remem` 子串但不是 remem-owned invocation 的第三方 hook |
| repair preserves mixed hook arrays | 单元 | 同一 event/matcher object 的 `hooks` array 同时含 remem 和第三方 command 时，只移除/替换 remem inner command，第三方 sibling 保留 |
| repair removes hostless legacy hooks | 单元 | `/tmp/remem context`、`remem summarize` 等 hostless legacy remem hooks 被移除并替换为 host-aware hooks |
| repair idempotent | 单元 | 连续 repair 两次后 hook 数量不增加 |
| repair dry-run no write | 单元 | 文件内容不变，输出 repair plan |
| repair has no install side effects | 单元 | 允许只读 `.claude.json` MCP 诊断，但不写 `.claude.json` MCP、不清理 settings legacy MCP、不初始化 runtime store、不创建 API token |
| doctor after repair | 单元 | MCP healthy/absent fixture 中 doctor hook check 为 Ok 且详情含 5/5；stale MCP fixture 中 doctor 继续报告 drift |
| invalid settings fails closed | 单元 | unreadable、根节点非 object、JSON 非法或写入失败时 repair 返回 error，路径上下文可诊断 |

## 8. 验证命令

- `cargo test hook_integrity -- --nocapture`
- `cargo test install:: -- --nocapture`
- `cargo test doctor:: -- --nocapture`
- `cargo test context:: -- --nocapture`
- `cargo fmt --check`
- `cargo check`
- `cargo test`
