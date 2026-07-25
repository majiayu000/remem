# Product Spec

Status: Draft，等待人工批准；不得据此开始实现
Date: 2026-07-23

## Linked Issue

GH-824

- Epic: GH-821
- 前置依赖: GH-823 定义 canonical Cursor host identity 与 hook payload/command 协议；
  GH-824 自己定义并拥有安装侧 `hosts.cursor` defaults/normalization、receipt 与清理合同
- 实测阻塞项: GH-822 的 Cursor 3.12.17 evidence PR #914（exact head
  `c0802c42c3fc22770aecb0b7b2eec88f117f795c`）已合并；本 packet 对该证据的采用仍等待
  fresh exact-head 人工批准；该证据已证明
  event/payload/部分 context 行为，但没有证明统一的“hooks effective”
- 当前工作流状态: `ready_to_spec`

`tasks.md` 可以随本 Draft 一起记录依赖关系、人工门和后续实现顺序，但它不是
runtime 实现授权。GH-822 real-host evidence、GH-823 批准且落地，以及 GH-824
人工批准三项全部满足前，所有 implementation task 都保持 blocked。

## 用户问题

remem 目前只能为 Claude Code 和 Codex 安装 MCP 与 hooks。Cursor 用户即使已
安装 remem，也没有受支持的 `--target cursor`、可靠的卸载路径或可诊断的配置
状态。直接套用现有 JSON 修改逻辑还会带来更严重的风险：合法但 shape 错误的
配置可能被覆盖，名称中偶然包含 `remem` 的用户 hook 可能被删除，两个 Cursor
配置文件可能只更新一个，以及临时文件可能短暂暴露用户 MCP 配置中的 secret。

本 issue 定义只管理用户级 `~/.cursor/hooks.json` 与
`~/.cursor/mcp.json` 的安装面，并把不能证明安全或有效的状态显式阻断。

## 目标

- 提供 `install`、`uninstall`、`--hooks-only` 与 `--dry-run` 一致的 Cursor
  目标语义。
- 只增删精确定义的 remem managed entry，并在 JSON 语义上保留 plan snapshot
  以及每次 final comparison 时已经可观察到的 foreign 数据；comparison→rename
  窗口内不可观察的非协作编辑遵守 B-012 的明确例外。
- 对 malformed、schema drift、managed collision、final comparison 可检测的并发编辑和
  可观察的多目标 partial state fail closed，且由 doctor 给出可恢复诊断；不得声称能对
  B-012 的 post-comparison/pre-rename 窗口 fail closed。
- 仅在 GH-823 的 canonical `cursor` host/hook 合同可用时安装；GH-824 自己实现并
  版本化安装侧 `hosts.cursor` defaults/normalization 与 receipt，绝不创建或依赖
  `hosts.unknown`。

## 非目标

- 不写任何项目级 `<project>/.cursor/hooks.json` 或
  `<project>/.cursor/mcp.json`。
- 不管理 Team/Enterprise hooks，也不改变 Cursor 的来源优先级。
- 不在本 issue 定义 hook stdin/stdout、context 注入或 transcript 解析；分别由
  GH-823、GH-822 与 GH-825 管理。
- 不迁移 Cursor 内建 Memories，不保存 PoC 原始 payload，不修改 Claude/Codex
  安装合同。
- 不承诺两个独立文件具备操作系统级事务原子性。

## Behavior Invariants

1. B-001 `remem install --target cursor` 只管理用户级
   `~/.cursor/hooks.json` 和 `~/.cursor/mcp.json`。在已有
   `~/.cursor/` 的 macOS/Linux 上，`--target auto` 包含 Cursor；在当前没有获准
   command renderer 的 Windows/UNC 平台上，`auto` 跳过 Cursor并输出不含配置内容的
   non-fatal diagnostic，继续安装已检测到的 Claude/Codex host。
   `--target cursor` 与 `--target all` 是显式请求，只要其选择集包含无法渲染的
   Cursor 就在任何 host 写入前整体 fail closed。`--target claude` 与
   `--target codex` 不读写 Cursor 文件。
2. B-002 Cursor 安装依赖 GH-823 的 canonical host identity 与 closed-set hook-host
   parser；安装侧 runtime mapping、`hosts.cursor` defaults/normalization、
   `install_receipt` schema 及其清理/兼容策略全部由 GH-824 定义并拥有，不是 GH-823
   的隐含交付物。安装前必须先确认当前 remem runtime 已实现批准的 GH-823 identity，
   且当前 binary 支持 GH-824 的 install contract version；任一不满足时安装整体失败。
   首次安装时用户 config 尚无 `hosts.cursor` 是正常状态：两个 Cursor 文件通过
   preflight 后，`ensure_config_for_hosts` 必须用 GH-824 批准的 defaults materialize
   canonical `hosts.cursor`。contract v1 的精确缺省值是
   `memory_profile = "codex"`、`context_gate = "strict"`、
   `context_color = true`、`capture_adapter = "cursor"`；只补缺失字段，不覆盖
   类型和取值合法的显式用户值，但 `capture_adapter` 是 host identity boundary，只允许
   精确字符串 `cursor`；`claude-code`、`codex-cli`、`unknown` 或其他值即使类型正确也
   必须 preflight fail closed。其他字段的错误类型/非法闭集值同样失败。该 config 写入
   纳入 B-010..B-012 的 rollback 边界；缺少用户 section 不得误报为 unsupported。
   任何路径都不得创建、读取或把成功归因于 `hosts.unknown`。
3. B-003 新生成和已有的 `hooks.json` 顶层必须是 object，且 `version` 必须是
   JSON integer `1`，`hooks` 必须是 object。校验器必须遍历整个 version-1
   文档，而不只检查 remem managed event。本 packet 冻结的 Cursor hooks v1
   event 闭集恰为：
   `beforeShellExecution`、`beforeMCPExecution`、`afterShellExecution`、
   `afterMCPExecution`、`beforeReadFile`、`afterFileEdit`、
   `beforeTabFileRead`、`afterTabFileEdit`、`stop`、
   `beforeSubmitPrompt`、`afterAgentResponse`、`afterAgentThought`、
   `sessionStart`、`sessionEnd`、`preCompact`、`subagentStart`、
   `subagentStop`、`preToolUse`、`postToolUse`、`postToolUseFailure`、
   `workspaceOpen`。每个 event value 必须是 array，每个 entry 必须精确匹配以下
   一个 variant：
   - command entry：`type` 缺失或精确为 `"command"`，required non-empty string
     `command`，不得有 `prompt`；
   - prompt entry：required `type: "prompt"` 与 non-empty string `prompt`，
     不得有 `command`；
   - 两种 variant 都只可额外带 positive finite number `timeout` 与 boolean
     `failClosed`；`matcher` 仅可在 `preToolUse`、`postToolUse`、
     `postToolUseFailure`、`subagentStart`、`subagentStop`、
     `beforeShellExecution`、`afterShellExecution`、`beforeReadFile`、
     `afterFileEdit`、`beforeSubmitPrompt`、`stop`、`afterAgentResponse`、
     `afterAgentThought` 上出现，且必须是 non-empty string；`loop_limit` 仅可在
     `stop`/`subagentStop` 上出现，且必须是 positive integer 或 JSON `null`。
   除上述字段外无其他字段。未知 event、未知 entry shape、字段缺失、额外字段、
   错误类型、`version` 缺失/非整数/非 `1` 均为 schema error，原文件不变。
   符合该冻结 schema 但不匹配 remem ownership 的 foreign entry 必须按 B-008
   原样语义保留；官方 schema 后续漂移需要新的 contract version，不得在 v1
   parser 中猜测放行。
4. B-004 新生成和已有的 `mcp.json` 顶层必须是 object，`mcpServers` 必须是
   object；校验器必须遍历每个 server value，不能只校验 `mcpServers.remem`。
   Cursor MCP v1 foreign entry 必须精确匹配以下一个 variant：
   - explicit stdio：required `type: "stdio"` 与 non-empty string `command`；optional
     `args` 是 string array，optional `env` 是 string-to-string object，optional
     `envFile` 是 non-empty string；
   - documented-example stdio compatibility：`type` 缺失，required non-empty string
     `command`，且 optional 字段与 explicit stdio 相同；该 variant 只用于校验/保留
     foreign entry，remem managed builder 不得生成；
   - remote：required non-empty string `url`；optional `headers` 是
     string-to-string object，optional `type` 只能是 `"http"` 或 `"sse"`；optional
     `auth` 是 object，其中 required non-empty string `CLIENT_ID`，optional non-empty
     string `CLIENT_SECRET`，optional `scopes` 是 non-empty-string array，且无其他字段。
   两个 variant 不得混用字段，且都不允许上述闭集之外的字段。server value 非 object、
   缺少 transport identity、混合 command/url、字段类型错误或未知字段时，整个操作在
   写前 fail closed，两个原文件均保持不变。该冻结 schema 对齐 2026-07-23 可审查的
   Cursor 官方 stdio/remote 配置与官方 staff `envFile`/typed-HTTP 示例；后续 schema
   漂移必须新增 contract version，不能因 entry 属于 foreign server 就跳过校验。
   Cursor contract v1 的 `mcpServers.remem` 必须精确等于
   `{"type": "stdio", "command": "<binary_path>", "args": ["mcp"]}`：`command` 是未作 shell
   拼接的 canonical binary path 字符串，`args` 是精确单元素 array，不含
   `transport`、`env` 或其他字段。该 shape 对齐 Cursor 官方 stdio 详细字段表中
   required `type: "stdio"` 与 command/args 形式，并由 Cursor 专用 builder 产生；
   不得直接复用可能携带其他 Claude 专属字段的 `build_mcp_server`。缺失容器可以创建；已有
   容器类型错误时失败，不得覆盖。
5. B-005 GH-824 自己拥有安装侧 exact builder；GH-823 只提供 runtime subcommand、
   payload parser 与 capability contract。Cursor v1 managed entry 只允许
   `{"command": <string>, "timeout": <positive integer seconds>}`，不包含 `matcher`、
   独立 `args` 或其他字段。`<binary_path>` 是 receipt 绑定的唯一 typed path slot。
   在已验证的 macOS/Linux contract v1 中，command 通过当前安装器的 POSIX
   `shell_quote` 规则渲染：只含 ASCII alphanumeric、`/._-` 时原样输出，否则整体
   单引号包裹并把每个 `'` 替换为 `'\''`；随后追加固定 command tail。receipt 同时
   记录 canonical unquoted path 和最终 rendered command/digest。Windows/UNC 在
   #822 提供并人工批准 argv/renderer fixture 前 fail closed，不得套用 POSIX quoting。
   待本 packet 人工批准的 contract v1 shape 为：
   - observe 是一个不可拆分 bundle。只有 GH-823 批准并实现覆盖 Cursor 全部可能
     delivered failure tool names 的 total policy 后，才同时安装 `postToolUse` 与
     `postToolUseFailure` 的
     `"<binary_path> observe --host cursor"` entry（各自 `timeout: 120`）。该 policy
     必须对已批准 failure shape 进行 bounded capture；对未批准但结构合法的
     `postToolUseFailure` tool variant 明确执行 zero
     capture/enqueue/spill/adapter/database write，并写不含 payload 的 error-level
     unsupported diagnostic。该 total-failure policy 不得覆盖成功
     `postToolUse`：结构合法的未知 non-MCP `tool_name` 仍按 GH-823 generic contract
     verbatim capture；只有 B-016 选择 MCP-specific ownership 时，已验证的 generic
     MCP 成功投递才执行 zero-write。若 GH-823 只实现 observed failed-Read 或
     尚未覆盖 failure precedence，则整个 observe bundle 缺席；不得安装
     success-only capture；
   - B-016 只有在 #822 先实测并由 GH-823 exact-head 人工批准 specific payload 的稳定、
     opaque per-call ID（覆盖同 generation 两次同 tool 调用与各自 replay）后，才可选择
     MCP-specific ownership；PR #914 的 specific payload 没有该 ID，不能用
     server/tool/generation/input/result/duration 派生键替代。在该前置满足并选择
     MCP-specific ownership 时，额外为
     `afterMCPExecution` 安装同一 observe command 和 `timeout: 120`；
     `beforeMCPExecution` 不注册。通用 `postToolUse` 仍服务非 MCP 工具，但
     GH-823 runtime 必须
     对其中 `tool_name` 为已验证 `MCP:` variant 的 generic delivery 成功返回且
     zero capture/enqueue/spill/adapter write，由 specific event 的 single-capture/upsert
     路径独占；前置未满足时 generic ownership 是唯一可安装分支，specific events 不注册且
     generic MCP 不 drop；
   - GH-825 reader/runtime capability 成为 proven 后，`stop` 安装
     `"<binary_path> summarize --host cursor"`、`timeout: 120`；
   - Cursor 3.12.17 的 `sessionStart` injection 为 blocked，`preCompact` action 未获产品批准，
     所以 contract v1 均不安装；不得用候选 event 填空。
   timeout 是 GH-824 的待人工批准安装 policy，不是由 probe 的 `timeout: 1/10` 推导。
   Cursor 3.12.17 实测表明默认 non-zero/timeout hook 不阻断 prompt 且 UI 不显示错误；
   foreign v1 entry schema 允许 `failClosed`，但 remem managed builder v1 不生成该
   字段，也没有实测依据声称它能保护 capture。因此“fail closed”只描述 remem 自身边界：
   malformed payload、未知 event 或 event/command mismatch 必须 non-zero、error-level
   log、zero remem side effects。唯一不同的已安装分支是 B-005 的结构合法但 tool name
   未批准的 `postToolUseFailure`：它必须 exit 0 explicit no-op，同时写无 payload 的
   error-level diagnostic，并保持 zero remem side effects。该例外不能放宽 malformed
   shape 或其他 event。以上均不宣称 Cursor host 会阻断 prompt。doctor 和安装输出必须明确报告
   `hook_failure_policy: host_continues`，不得把自动 capture failure protection 标为
   proven；人工批准本 packet 是接受这一已知 host-level residual risk，不是消除它。
   GH-822/PR #914 已在 Cursor 3.12.17 真实触发
   `sessionStart`、`postToolUse`、`stop` 与手动 `/summarize` 的 `preCompact`，并记录
   sanitized payload shape；这只允许 GH-823 amendment 冻结对应协议，不直接授权 builder
   写入。未经人工采用的 event/字段仍只能列为候选。事件被调用与 remem context 对模型
   effective 是两个独立 gate，不能由序列化测试或 hook stdout 推导。
6. B-006 remem-owned MCP 项只能通过 key 精确等于 `mcpServers.remem`，再满足以下
   之一识别：(a) value 精确匹配当前 managed builder；(b) value 的批准版本化
   type/command/args shape、canonical unquoted old binary path 与整项 canonical JSON digest
   均和 `hosts.cursor.install_receipt` 中同 component、contract version、旧 binary path
   的记录精确一致；(c) 精确命中文档化的有限 legacy receipt/allowlist。receipt 是
   remem-owned、非敏感、版本化记录，至少包含 install mode、contract version、
   installed binary path 与每个 managed component 的 canonical digest。这样 binary
   path 改变后仍可更新已记录的旧自有项，但 key、command 或 basename 单独命中都不
   构成 ownership。receipt 缺失、被修改、摘要不符或任何其他同名 value 都是
   collision，安装和卸载均失败。
7. B-007 remem-owned hook 使用 current-builder exact equality，或由 receipt 证明的
   old-path exact equality；后者要求 event、component ID、command tail、timeout、
   contract version、旧 binary path 与整项 digest 全部匹配。hook entry 内没有
   structural marker、matcher 或独立 args。不得使用字符串 `contains("remem")`、
   任意 executable basename、路径片段、任意未知 subcommand 或 path wildcard 删除
   用户条目。executable path 是唯一 typed slot；旧值仍必须等于 receipt 记录，
   其他字段和整项 digest 必须精确。疑似但
   证明不完整的条目是 collision，安装和卸载均失败。
8. B-008 对同一已验证 plan snapshot，foreign JSON 的语义必须保留：除本次精确
   managed entry 的添加、替换或删除外，解析后的所有 foreign key、value、array 顺序和
   JSON 标量均相等；B-012 final comparison 已观察到的新 external version 也必须原样保留并
   abort。该保证止于每次 final comparison，不包含 comparison 返回相等后、rename 前才发生
   且随后被覆盖的不可观察编辑。格式、空白和 object key 排列不承诺 byte-for-byte 保留。
9. B-009 同一操作必须先读取并完整校验两个 Cursor 文件与 canonical runtime
   config/receipt，再规划任何写入。显式 Cursor/All 以及 Auto 选中 Cursor 时，完整
   `CursorConfigPlan` preflight 必须发生在 `ensure_runtime_store_ready()`、runtime host
   materialization、API token 或任何 host 配置写入之前；Auto 的 unsupported-Cursor skip
   同样在这些副作用前决定。任一输入 malformed、schema 不兼容或发生
   collision 时，全部目标都保持原样，命令非零退出并明确指出路径与错误类别，
   不得把失败显示为成功，也不得在首次运行留下新 key/database/token/runtime config。
10. B-010 apply 使用“两份 Cursor 文件 + canonical runtime config/receipt 的 staged
    apply 与可验证 compensating rollback”。每个目标单独原子替换；后续替换失败
    时尝试从已验证 snapshot 恢复已经提交的目标。receipt 只能与最终成功的
    full/hooks-only mode 和 managed digests 一起提交，不能预先宣称成功。输出不得把
    该机制称为真正的跨文件 atomic transaction。
11. B-011 若 compensating rollback 失败，命令必须非零退出并显式报告
    `partial_state`、可能已改变的路径和 `remem doctor` 修复指引；不得吞掉
    rollback error、仅记录 warning 或继续后续安装步骤。
12. B-012 snapshot 包含两个 Cursor 文件和 canonical runtime config/receipt 的
    存在性、内容与用于检测 concurrent edit 的 identity。每次 replace 或 rollback
    restore 紧邻执行前必须做最终比较；在该比较时已经可观察到的外部编辑必须使操作
    非零失败，并保留该版本，不得用 plan/snapshot 覆盖。普通可移植文件系统上的
    compare 与 rename 不是 CAS：非协作进程若在最终比较完成后、rename 前写入，
    该编辑仍可能被本次 rename 覆盖；本合同不承诺保留或检测这个窗口内的编辑，也
    不得把 advisory lock 宣称为互斥保证。replace 后 read-back 若观察到 drift，或
    因此无法安全 rollback，必须非零返回 `partial_state`、可能改变的路径和 doctor
    指引；若 post-compare 编辑先被 rename 覆盖而 read-back 只看到本次 planned
    bytes，则它可能不可检测，这是必须在文档与测试中保留的 residual user-data-loss risk。
13. B-013 涉及完整配置内容（包括 foreign MCP `env`）的每个 fresh 临时文件，
    必须在写入任何 byte 或 secret 前无条件成为 owner-only：Unix 使用 `0600`
    并移除继承/default ACL，其他平台使用可证明等价的 owner-only 权限；平台无法
    提供该保证时 fail closed。不得先继承现有目标可能宽松的 mode/ACL，也不得在
    写入后才收紧权限。replace 后目标继续保持 owner-only，不恢复宽松权限；写入后
    完成 file sync、replace 与适用平台的 parent-dir sync。失败时清理临时文件；
    日志、错误、dry-run 与 doctor 不输出 secret 值或完整配置。
14. B-014 重复 install 必须收敛到同一 JSON 语义，既不重复 managed entry，
    也不改变 B-008/B-012 保证边界内的 foreign data。重复 uninstall 在没有 managed
    entry 时是成功 no-op；用户创建的配置文件不会仅因移除最后一个 remem entry 而被删除。
15. B-015 `install`、`uninstall`、`--hooks-only` 和 `--dry-run` 使用同一解析、
    schema 与 collision 校验器。`--hooks-only` 仍校验并显示两个 Cursor 路径，
    但只把 hooks 文件列为 would-change；MCP 文件必须显示为 validated/no-change。
    成功 apply 后 receipt 必须精确记录 `mode: hooks_only` 和已安装 hook digest；full
    install 记录 `mode: full` 及两类 digest，uninstall 成功后清除该 managed receipt。
    receipt 写失败必须使命令失败并走 B-010 rollback，不能留下“成功但无意图证据”的
    单文件状态。
16. B-016 Cursor dry-run 在零副作用前提下显示两个绝对路径，以及每个路径的
    `add`、`remove`、`replace`、`no-op` 或 `would-fail` 结果与非敏感原因。
    dry-run 不创建目录、文件、临时文件、runtime config、key、database 或 token，
    也不改变 mtime/permissions。
17. B-017 uninstall 先用同一严格解析器识别 managed/legacy 条目，再删除精确
    ownership 命中的项。遇到 malformed、collision、concurrent edit 或另一
    文件不可安全更新时，遵循 B-009..B-013，不做模糊清理。
18. B-018 doctor 分别报告 Cursor application `detected`、配置是否
    `configured`、`configured_mode: full|hooks_only|none|unknown`、`malformed`、
    `partial_state`、managed `drift`、ownership `collision`、hook `effective`，以及
    command capability `session_init: unsupported`。human-readable 输出必须包含精确
    行 `session-init: not supported on cursor`；该 capability 是 GH-823 的批准合同，
    不得因未检测到 Cursor、未配置、hooks-only 或 `effective` 未证明而被省略或伪装为
    supported。
    hooks 存在、MCP 缺失且 receipt 的 mode/path/digest 全部匹配时是成功的
    `hooks_only`，`partial_state` 必须为 false，doctor/repair 不得建议补写或删除 MCP。
    相同文件 shape 若 receipt 缺失、mode 不符或 digest 不符则是 ambiguous/broken
    `partial_state`，不得猜成 intentional。以上维度不可折叠为一个“installed”布尔值；
    目录存在不等于 configured，shape 可解析不等于 effective。
19. B-019 `effective` 只可由 GH-822 在真实 Cursor 中记录的、版本和 capability
    匹配的 real-agent evidence 判定；静态 JSON、hook stdout 或日志不能证明 context
    已对模型生效。PR #914 在 Cursor 3.12.17 证明 `postToolUse.additional_context`
    这一 host delivery 能力对后续模型可见，但本 packet 没有批准或安装 postToolUse
    context producer；doctor 必须把 `postToolUse_delivery: proven` 与
    `postToolUse_managed_context: not_configured` 分开，不能把 observe capture entry
    当成 context output。同一版本的 `sessionStart.additional_context` marker 未被模型看见，
    因而该 capability 必须为 `blocked`。`stop`/`preCompact` 只观察到 invocation，模型侧
    outcome 仍为 `unknown`。在独立 packet 批准并安装 postToolUse context command/output
    前，它不进入 managed install/effective 成功模型。doctor 必须逐 capability 报告这些状态；
    只要安装合同要求的任一
    capability 未 proven，汇总 `effective` 就不得为 proven，安装输出不得宣称自动记忆有效。
    同一 evidence 还要求 doctor/JSON 报告
    `hook_failure_policy: host_continues`：它与 parser 的 remem-side zero-write
    fail-closed 是不同维度，不能折叠成 `effective` 或改写成 host-blocking。
20. B-020 rollback 顺序是先执行当前版本的 `uninstall --target cursor` 清理
    managed entry，再降级 remem。若旧版本无法识别新 shape，doctor 必须先给出
    当前版本修复/清理指引；不得建议先降级后盲删。
21. B-021 Cursor 官方文档事实、GH-822/PR #914 已观察事实和剩余未知必须分开记录。
    文档事实可以决定 schema 校验边界；真实事件 payload、context 可见性、大小限制、
    background 行为和 `preCompact` 实效只有版本化 PoC evidence 经人工采用后才能升级为
    实现合同。单一 event/capability 的 proven 结论不得外推到其他 event、版本或 host mode。
22. B-022 Claude Code 与 Codex 的 install/uninstall/dry-run/doctor 行为保持不变。
    Cursor 的错误不得被当作其他 host 的成功，multi-host 操作中的失败与
    partial state 必须逐 host 可见。

## 官方文档事实（2026-07-23）

以下是 Cursor 官方文档可直接支持的配置事实，不代表 GH-822 PoC 已通过：

- hooks config 使用 `version: 1` 与 `hooks` object；B-003 固定官方当前 21-event
  lower-camel 闭集、command/prompt entry discriminator、共同
  timeout/failClosed、event-specific matcher 与 stop/subagentStop loop_limit。
- 用户级 hooks 路径是 `~/.cursor/hooks.json`，项目级路径是
  `<project>/.cursor/hooks.json`；本 issue 只管理前者。
- Cursor 监听 hooks config 并在保存时自动 reload；无法加载时官方排障建议
  restart Cursor。产品输出因此可陈述“文档称会自动 reload”，但 effective 仍
  受 B-019 约束。
- 用户 hook command 从 `~/.cursor/` 运行；项目 hook 从 project root 运行。
  本 issue 的命令不得假设 project cwd。
- 来源优先级是 Enterprise → Team → Project → User；各来源的 matching hooks
  都会运行，冲突 response 按该优先级 merge。用户级配置不等于最终最高优先级。
- 用户级 `~/.cursor/hooks.json` 不会出现在 cloud agent VM；project、Team 与
  Enterprise hooks 才有各自的 cloud 分发路径。
- MCP 用户级路径是 `~/.cursor/mcp.json`，root key 是 `mcpServers`。

Sources:

- https://cursor.com/docs/hooks （访问于 2026-07-23）
- https://cursor.com/docs/mcp （访问于 2026-07-23）

## GH-822 / PR #914 实测事实与剩余未知

PR #914 exact head `c0802c42c3fc22770aecb0b7b2eec88f117f795c` 已合并（merge
commit `8ba201ea4448e778a9ee4f84d5ff757b6c538d13`），在 Cursor IDE 3.12.17、
隔离合成 workspace 和真实 Agent 中记录了脱敏 evidence。该 evidence 已落库，但本
packet 对它的协议采用仍须 fresh exact-head 人工批准；以下事实只有经该批准并进入
GH-823 amendment 后才能成为 managed install contract：

- 新 Agent 的第一条 prompt 触发 `sessionStart`；仅启动应用不触发，退出应用未观察到
  `sessionEnd`。`sessionStart.transcript_path` 为 `null`，并含
  `is_background_agent: false`、`model_id` 与 `model_params`。
- `beforeSubmitPrompt`、`beforeShellExecution`/`afterShellExecution`、`preToolUse`、
  `postToolUse`、`postToolUseFailure`、`subagentStart`/`subagentStop`、`stop` 与
  手动 `/summarize` 的 `preCompact` 均被真实触发；配置保存后无需重启即可识别。
- foreground parent session 后续 event 使用稳定 JSONL `transcript_path`；第一批
  `beforeSubmitPrompt`/`beforeToolUse` 可为 `null`。inner `Task` subagent 使用不同
  session identity，tool event 和 `subagentStop.agent_transcript_path` 可为 `null`。
- 实测 tool names 包含 `Read`、`Shell`、`Task`、`MCP:browser_tabs`；MCP
  `tool_input`/`result_json` 为 JSON string。失败 `Read` 触发
  `postToolUseFailure`，含 `failure_type: error`、`is_interrupt: false`。
- 完成 Stop 为 `status: completed`；取消 response 为 `status: aborted`。两者
  `loop_count` 都观察为数字 `0`；完成 Stop 有 token 字段，取消 Stop 无 token 字段。
- `postToolUse.additional_context` marker 对后续真实模型可见；同一版本
  `sessionStart.additional_context` marker 未被模型看见。因此只能把前者标为
  `proven`，后者为 `blocked`，不能宣称统一 effective。
- 默认非零退出和 `timeout: 1` 的慢 hook 未阻断 prompt，UI 也未显示错误；该样本未测试
  fail-closed 配置，不能据此冻结生产 timeout/failure policy。

仍未证明：context 最大值及截断/拒绝边界、background/CLI/multi-root、Windows/UNC、
Team/Enterprise/project source 组合、`sessionEnd` 可靠性、`stop`/`preCompact` 的模型侧
outcome，以及缺失/null/非零 `loop_count` 和 `status: error`。这些未知不得由当前样本外推。

## 验收标准

- [ ] `specs/GH824/product.md`、`specs/GH824/tech.md` 与
      `specs/GH824/tasks.md` 通过 SpecRail packet 检查，且仍处于等待人工批准
      状态；task plan 仅记录依赖和顺序，不授权 runtime 实现。
- [ ] B-001..B-022 每项都在 tech spec 中映射到具体实现面和确定性验证。
- [ ] malformed、whole-document wrong-shape/unknown event、collision、首次
      精确 `hosts.cursor` defaults materialization、Cursor MCP
      `type:stdio` exact shape、preflight-failure store/key/db/token zero-side-effect、
      space/single-quote path rendering、specific-ownership generic-MCP zero-write、
      旧 binary path receipt upgrade、intentional
      hooks-only 与 broken partial 区分、后续目标写失败、rollback 失败、concurrent
      edit 的 final-comparison 前保留、post-comparison/pre-rename residual overwrite
      窗口与 post-rename read-back drift、现有宽松权限目标的 secret temp
      permission/ACL、doctor 的
      `session_init: unsupported`/精确 human-readable line 与 dry-run 零副作用都有
      确定性测试；用户文案不得把前两者统称为“所有并发编辑都 fail closed”或“所有 foreign
      数据都保留”。
- [ ] GH-822/PR #914 exact-head real-host evidence 已获人工采用，并通过 GH-823
      amendment 决定 managed event shape 与逐 capability effective gate；GH-823 已获批准且
      其 canonical `cursor` host/hook 合同已经
      实现；GH-824 product/tech 已获人工批准并通过 implementation route gate。三项
      全部满足后才能开始任何 implementation task。
- [ ] `cargo fmt --check`、`cargo check` 与 `cargo test` 在实现 PR 的当前 head
      上通过；本 spec-only PR 不以旧输出代替未来验证。

## 边界情况

| Category | Verdict |
| --- | --- |
| Empty / missing input | covered: B-001, B-003, B-004, B-014 |
| Error and failure paths | covered: B-009..B-013, B-017 |
| Authorization / permission | covered: B-013；用户级文件也必须 owner-only 处理 secret-bearing temp |
| Concurrency / race / ordering | covered: B-009, B-010, B-012；只保证最终比较时可观察编辑的保留，compare→rename residual race 明示 |
| Retry / repetition / idempotency | covered: B-014 |
| Illegal state transitions | covered: B-002, B-005, B-011, B-019 |
| Compatibility / migration | covered: B-006..B-008, B-020, B-022 |
| Degradation / fallback | covered: B-011, B-018, B-019；未知不得伪装成功 |
| Evidence and audit integrity | covered: B-016, B-018, B-019, B-021 |
| Cancellation / interruption / partial completion | covered: B-010..B-013 |

## 发布说明

实现发布时必须说明：只管理用户级 Cursor 配置、foreign JSON 仅承诺 plan snapshot 与
final-comparison 时可观察版本的语义保留、
Cursor 文件与 runtime receipt 的协调更新采用补偿回滚而非真正事务、用户 hook
不适用于 cloud agent、compare→rename 不是 CAS 且不保证保留该窗口内的非协作编辑，
以及降级前要先用当前版本 uninstall。若 GH-822 未证明 effective，功能不得以
“Cursor 自动记忆已启用”发布。
