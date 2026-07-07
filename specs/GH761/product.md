# GH761 Product Spec: Claude hook integrity self-heal visibility

Issue: https://github.com/majiayu000/remem/issues/761
Route: write_spec
Locale: zh-CN
Status: Draft for SpecRail approval (2026-07-08)

## 1. 背景

2026-07-07 的真实环境显示 Claude Code 的 `~/.claude/settings.json` 只保留了 remem 5 个 hook 中的 3 个：`SessionStart context`、`UserPromptSubmit session-init`、`PreCompact summarize`。`PostToolUse observe` 与 `Stop summarize` 被外部设置编辑丢失，第三方 hook 仍存在。

这导致自动 capture 链路停止近 3 周：工具写入不再进入 `observe`，会话结束不再触发 `summarize`，`session_rollup -> user_context_candidate` 也随之停止。`remem doctor` 已能报告 `Hooks (claude) stale or incomplete: 3/5 registered`，但需要用户主动运行 doctor；实际使用中，SessionStart 仍能输出上下文，容易掩盖 capture 已经断链。

## 2. 目标

P1. Claude `SessionStart` 上下文输出必须做轻量 hook integrity 自检。当 Claude hook 注册不完整或 stale 时，当前这次 hook 输出中必须出现可见 warning。

P2. Warning 必须说明实际注册数量、期望数量、受影响 host、配置文件位置或修复命令，并指向显式修复命令：`remem install --target claude --repair`。

P3. 新增显式修复模式：`remem install --target claude --repair`。它只修复 Claude hook 注册，不删除或改写第三方 hook，不依赖用户手工编辑 JSON。

P4. Repair 必须幂等。对已经完整的 Claude hooks 重复执行不产生重复 remem hook；对缺失 `PostToolUse` 与 `Stop` 的配置执行后恢复 5/5。

P5. `remem doctor` 现有检测能力保留，并在修复后报告 Claude hooks 5/5。

P6. 修复不能静默吞掉配置读取、解析或写入错误。配置不可读、JSON 非法、根节点不是 object、写入失败时必须返回失败并带路径上下文。

## 3. Non-Goals

N1. 不在 `SessionStart` 自动修改 `~/.claude/settings.json`。运行时只提示，不做隐式写入。

N2. 不修复或重排第三方 hook，不治理其他工具写入策略。

N3. 不改变 Codex hook 策略。Codex 继续按现有 2 个 hook 规则检测。

N4. 不把 repair 扩展为完整 doctor autofix 框架。首版只覆盖本 issue 的 Claude hook 注册修复。

N5. 不改变 capture、summarize、context 的核心数据语义。

## 4. 行为不变量

B1. 完整的 Claude hook set 是 5 个事件：`SessionStart`、`UserPromptSubmit`、`PostToolUse`、`PreCompact`、`Stop`，且分别调用当前 remem binary 的 `context`、`session-init`、`observe`、`summarize`、`summarize`，host 为 `claude-code`。

B2. Runtime warning 必须是可见的人读文本，出现在 Claude `SessionStart` context 输出中；缺失 capture hook 不能只写日志。

B3. Runtime warning 检测失败时不能破坏原 context 输出。无法读取用户配置时输出 warning；内部自检代码崩溃不得导致 context 全空。

B4. Repair 只能移除/替换 remem 自己的 hook entries，然后合并当前版本期望的 remem hook entries；非 remem entries 必须按 JSON value 语义保留。允许整体 JSON 格式化变化，但不能删除、改写或重排第三方 hook 的事件、matcher、command、timeout 等字段值。

B5. Repair 写入后 `remem doctor` 与 runtime self-check 使用同一套 hook expectation，不允许 doctor 认为 5/5 而 runtime 仍报 stale，或相反。

B6. `--repair` 与 `--dry-run` 同时使用时只输出将修复的目标，不写磁盘。

## 5. 验收

A1. 测试 fixture 删除 Claude `PostToolUse` 与 `Stop` 的 remem entries，同时保留第三方 entries；执行 Claude SessionStart context 渲染后输出包含 hook integrity warning，并包含 `3/5` 与 `remem install --target claude --repair`。

A2. 对同一 fixture 执行 `remem install --target claude --repair` 后，`~/.claude/settings.json` 恢复 5 个 remem hook，第三方 entries 仍存在。

A3. Repair 连续执行两次后 remem hook entries 不重复。

A4. Repair 后 doctor 对 Claude hooks 报告 5/5。

A5. JSON 非法或写入失败时 repair 返回失败，不报告成功。

A6. Repair 对 unreadable settings、根节点非 object、JSON 非法、写入失败都返回失败，并在错误中包含配置路径或根因上下文。

A7. Repair 不触碰 `.claude.json` MCP、不初始化 runtime store、不创建 API token；测试通过文件存在性或写入 spy 证明无副作用。

A8. `cargo test` 全绿，相关 focused tests 覆盖 self-check、repair、doctor 兼容、第三方 hook 保留和 repair 副作用边界。
