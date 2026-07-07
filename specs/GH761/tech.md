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
- `--repair` 与 `--hooks-only` 同时出现时 repair 语义优先；两者都不触碰 MCP。

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
- 完整性判断必须按当前 remem binary 路径与 `--host claude-code` 校验，不只按 command 字符串包含 `remem`。
- Hook removal 必须使用同一 parser 判断 remem-owned invocation：executable file stem 是 `remem`，subcommand 是该 event 的 expected subcommand，host 是 `claude-code` 或 legacy env host 可归一到 `claude-code`。不能用 `command.contains("remem")` 或 path substring 删除 entries。

## 3. Runtime self-check

在 Claude `SessionStart` context 路径增加只读自检：

- 入口：`src/context/render.rs` 中 `generate_context_for_invocation` 的最终 `ContextGateDecision.output` 组装后、`context_stdout_for_invocation` 调用前。
- 触发条件：`invocation.host == HostKind::ClaudeCode` 且 source 是 SessionStart 类事件（startup/clear/compact/resume 或 Claude 传入的空 source，按现有 Claude hook 行为确认）。
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

## 4. Repair 实现

在 install runtime 增加 repair path，避免复用完整 install 的 runtime store 初始化副作用。

推荐实现：

```rust
pub fn install(target: InstallTarget, dry_run: bool, hooks_only: bool, repair: bool) -> Result<()>
```

当 `repair == true`：

1. 解析目标 host。
2. 对 Claude 调用专用 hook-only repair 函数，例如 `ClaudeHost.repair_hooks_only(&bin)`。不能调用现有 `ClaudeHost.install_hooks(&bin)`，因为该方法会清理 settings 中的 legacy MCP entry，违反 repair 不触碰 MCP 的副作用边界。
3. 该函数可以复用 `build_hooks(bin, HookStrategy::ClaudeCode)` 和 JSON read/write helpers，但不能复用当前 substring-based `remove_remem_hooks`。必须新增 parser-based removal，只移除当前 host/event 下可识别的 remem-owned expected hook entries，然后合并 fresh hooks。
4. 写入后立即调用共享 integrity evaluator；不是 5/5 则返回失败。
5. 输出明确结果：`hooks -> ~/.claude/settings.json (5/5 registered)`；若 settings repair 成功但 doctor 会因 stale MCP 继续把 expected executable 绑定到旧 binary，输出必须提示 stale MCP/install-path 仍需完整 `remem install --target claude` 或 doctor 修复，而不是声称整机健康。

Repair path 不应：

- 调用 `ensure_runtime_store_ready`。
- 写 `.claude.json` MCP。
- 清理 `~/.claude/settings.json` 中历史误写的 `mcpServers.remem` entry。
- 创建 API token。
- 删除第三方 hook entries。

## 5. Doctor 兼容

`src/doctor/environment.rs` 可继续负责 doctor report，但 hook count 应来自共享 evaluator 或共享 expectation helpers。

修复后 doctor 的 Claude hook check 在 MCP 缺失或 MCP 指向当前 binary 时必须继续输出 `5/5 registered in <path>`。如果 MCP 指向 stale binary，doctor 现有 drift 检测不能被削弱：hook check 可以继续按 MCP 期望 executable 报告不匹配，或新增清晰 detail 说明 hooks 已 repair 但 MCP/install path stale。部分缺失时现有 `Warn` 语义保留，但详情里的修复命令应升级为 `remem install --target claude --repair`。

## 6. 文档

更新 README 的 install/doctor/troubleshooting 片段：

- 说明 `remem install --target claude --repair` 用于恢复丢失 hook。
- 说明 SessionStart warning 的含义。
- 明确 repair 保留第三方 hook，不触碰 MCP/runtime store。

若 CLI help 文本包含 install flags，也要同步。

## 7. 测试计划

| 测试 | 类型 | 断言 |
|---|---|---|
| shared evaluator detects 3/5 | 单元 | 删除 `PostToolUse` 与 `Stop` 后 registered=3，missing 包含两个事件 |
| context warning visible | 单元/集成 | Claude SessionStart 输出含 warning、3/5、repair 命令；即使 context gate suppresses normal output 也能显示 warning；Codex JSON 输出不被污染 |
| repair restores hooks | 单元/集成 | repair 后 Claude settings 中 5 个 remem hook 存在 |
| repair preserves third-party hooks | 单元 | 非 remem entries 保留，包括 command/path 含 `remem` 子串但不是 remem-owned invocation 的第三方 hook |
| repair idempotent | 单元 | 连续 repair 两次后 hook 数量不增加 |
| repair dry-run no write | 单元 | 文件内容不变，输出 repair plan |
| repair has no install side effects | 单元 | 不写 `.claude.json` MCP、不清理 settings legacy MCP、不初始化 runtime store、不创建 API token |
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
