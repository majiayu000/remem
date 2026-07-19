# Tech Spec

## Linked Issue

GH-852

## Product Spec

[`product.md`](product.md)

## Evidence and Approval Gate

- 代码基线为 `origin/main@2dc41cb332ead83ff39f234444fc76fc50713f43`（remem
  `v0.6.11`）；本分支已 rebase 到该 revision。
- 2026-07-20 fresh duplicate check 未发现引用 GH-852 的 open PR 或匹配 remote branch；当前
  `spec/gh852-host-native-memory` 是唯一已发现的本地 spec 分支。
- SpecRail `workflow.yaml` 为 v0.2.1。fresh `write_spec` route gate 在 issue `ready_to_spec` 和
  `ready_to_spec` label 下返回 allowed；这只授权 product/tech 草案，不满足
  `spec_approval`、`ready_to_implement`、security decision 或任何 implementation/merge gate。
- `check_workflow.py --spec-dir specs/GH852` 当前无条件要求 `tasks.md`，因此在合法的
  `ready_to_spec` 阶段会报告 `missing tasks.md`；不得用空 tasks 绕过。当前以 repo check +
  `write_spec` route gate 校验草案，待批准后由 `specrail-plan-tasks` 生成真实任务，再要求
  spec-dir check 通过。
- issue 引用的 `docs/research/agent-memory-optimization-research-2026-07.md` 不在上述 main。
  不可变报告 revision 或 maintainer 明确认可的等价一手证据，是 spec approval 的前置条件。
- 本恢复提交不执行真实宿主 PoC、不访问真实 Claude/Codex 用户目录，也不创建 `tasks.md`。
  PoC 证据、目录所有权模型、隐私审查与 security decision 必须在实现授权前由人工核对。

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Import CLI | `src/cli/archive_types.rs`, `src/cli/actions/import.rs`, `src/cli/actions/pack_import.rs` | `ImportAction` 只有 backup/markdown；backup 是 `--best-effort` direct restore；顶层 `--pack --dry-run` 有 planner，但 clean pack apply 可直接新增 active memory | Codex 原生记忆需要独立 source，不能复用 best-effort 或 direct-active 语义 |
| Candidate governance | `src/memory_candidate/`, `src/memory/poisoning.rs`, `docs/ARCHITECTURE.md` | `external_content` 禁止 auto-promote，候选支持 pending/quarantine；当前 scanner 只保证已进入该链路的文本 | Codex 导入必须接到候选治理边界，且不能假设 source-taint 已覆盖所有衍生 surface |
| Claude native output | `src/context/claude_memory/paths.rs`, `runtime.rs`, `render.rs`, `index.rs` | remem 向硬编码 `~/.claude/projects/<slug>/memory/remem_sessions.md` 写最近 summary，并用普通文件写更新 `MEMORY.md` pointer；无 `autoMemoryDirectory` resolver/receipt | 接管会新增目录所有权、原子性、双写和 startup 交付问题 |
| Claude native input | `src/observe/native.rs`, `src/observe/hook.rs` | Write/Edit 命中 `.claude/projects/*/memory/*.md` 时可直接 `insert_memory_with_branch`；只排除 `MEMORY.md`，错误由调用方 warning 后继续 | 这是现存 direct-active 与 silent-degradation 风险；扩大目录前必须先 fail closed 或 no-go，并排除 remem 自摄取 |
| Claude install | `src/install/hosts/claude.rs`, `src/install/paths.rs`, install receipt/doctor modules | 安装会合并用户设置并记录可卸载状态，但没有 `autoMemoryDirectory` | PoC 后的 opt-in 配置必须沿用原子变更与回滚契约 |
| Codex install/hooks | `src/install/hosts/codex.rs`, `src/install/config.rs`, `README.md` | 安装会在 `~/.codex/config.toml` 设置 `[features].hooks=true`，并向 `~/.codex/hooks.json` 合并 SessionStart/Stop；Claude 策略才构造工具 observe hooks | 第三交付物是覆盖审计，不是 wrapper 迁移 |
| Codex plugin | `plugins/remem/README.md`, `plugins/remem/skills/remem/SKILL.md`, `plugins/remem/scripts/activate-codex.js` | plugin 加载 `.mcp.json` 后即有 MCP，但不会静默安装 hooks；显式 activation 委托 `remem install --target codex --hooks-only` | 审计需区分 plugin-only 与已激活状态，并验证 activation 与 core installer 等价 |
| Host paths/doctor | `src/install/paths.rs`, `src/doctor/` | 没有 Codex native memories path 或格式状态 | 需要可覆盖路径发现和未配置/错误/不支持诊断 |
| Security contracts | `docs/specs/memory-poisoning-defense/`, `specs/GH855/`, `docs/specs/README.md` | GH-672 是已实现 candidate/memory contract；GH-855 定义 source-evidence 与 summary sink 扩展，但当前 main 尚未实现该 runtime | GH-852 新来源不能把计划中的 GH-855 防线误写成现状；任何 active/read path 必须等待或独立满足同等契约 |
| SpecRail | `workflow.yaml`, `states.yaml`, `labels.yaml` | v0.2.1 的 `write_spec` 只从 `triaged`/`ready_to_spec` 产生 product/tech，implementation 需要 human spec approval 和 `ready_to_implement` | 本 PR 必须保持 spec-only、Draft、`Refs #852` |

## 设计方案

### 1. PoC 先行与证据包

implementation 授权前先在隔离 HOME 与 `REMEM_DATA_DIR` 下执行两个宿主 PoC，并把不可变结果
链接到 spec review；证据落点须在 PoC 阶段 search-first 后由 maintainer 确认，本恢复提交不预造
第三个 spec 文件。证据包括：

- 宿主二进制版本和设置来源；
- 初始目录树的路径脱敏清单与内容摘要；
- 可复现命令/交互步骤、退出码、事件名称和 schema 摘要；
- 写入前后摘要、并发/异常行为和清理后摘要；
- “观察事实”与“设计推断”分栏，以及不支持版本清单。

PoC 不读取或复制真实用户记忆；若宿主无法在隔离 HOME 工作，停止并请求新的人工授权，不把
真实配置当测试夹具。

### 2. Claude `autoMemoryDirectory` 所有权模型

以下是条件设计，不是已验证的宿主事实。只有 PoC 验证 effective settings、实际加载内容、
容量边界、hook failure propagation 与生命周期后，才可在现有 Claude host installer 中增加
显式 opt-in；任一关键假设不成立即记录 no-go，不落地接管：

1. 读取用户级/策略级/显式 settings 的有效值和来源；项目/local scope 一律拒绝。
2. 规范化绝对路径或 `~/` 路径，但不解析用户目录外的符号链接为可写目标。
3. 若已有非 remem 值，dry-run 报告 conflict；apply 不覆盖，除非未来 CLI 设计提供单独的显式
   adopt 动作并再次人工批准。
4. 使用现有设置合并、临时文件原子替换、备份和 install receipt 机制；保留所有未知 JSON 键。
5. 接管启用时，`sync_to_claude_memory` 可继续把扩展材料写入 effective directory 的
   `remem_sessions.md`，但 topic file 只供按需读取，不算 SessionStart 已交付内容。真正互斥的
   remem 正文必须直接写入 `MEMORY.md` 中带 generation 与 digest 的 marker-bounded delivery
   block，并完全位于真实 PoC 证明会在 startup 自动加载的窗口内。文档中的行数/字节数只能作为
   待验证候选约束，不能代替实际宿主证据。`ensure_memory_index` 改为只维护该 block，原子保留
   block 外内容；若加入 block 会截断、移出实测窗口或覆盖既有 startup-loaded 用户内容，则本项目
   保持 `hook_only` 并 fail-visible 报告容量原因。
6. 每次导出先生成 prepared manifest，精确列出实际渲染进 delivery block 的 stable memory
   id/content hash；只存在于 `remem_sessions.md` 的条目不在 manifest。新的 SessionStart 只有在
   effective setting 指向 receipt 目录、prepared manifest 与 delivery block 的 generation/digest
   匹配、且 block 经解析仍完整位于 startup window 时，才排除这些 IDs。
7. `MEMORY.md` delivery block 的原子替换是唯一 commit point，而不是跨文件“原子发布”：
   在无活动 Claude 会话的 maintenance window 内，先移除旧目录 remem block，保持 hook 全量
   注入；再依次原子写 effective setting、receipt、prepared manifest 和无 active block 的目标
   文件。此时任何 crash/restart 都仍是 `hook_only`。最后一次原子写只加入已准备好的 delivery
   block；下一次 SessionStart 看到匹配 block+manifest+setting 才进入 native delivery。
8. recovery 在每次 install、doctor 和 SessionStart 前读取 setting/receipt/manifest/block：
   无 block 是 `hook_only`；完整匹配是 `native_active`；任何其他组合都是 `inconsistent`，必须
   error 且阻止成功的 SessionStart，不得选择可能双开/双关的 fallback。真实 PoC 必须先证明
   hook 非零状态能阻止该会话；否则该 host/version 的 native bridge 为 no-go，不写 active block。
9. 回滚以原子移除 delivery block 作为反向 commit point；移除后 SessionStart 恢复全量注入，
   再清理 setting、prepared manifest、receipt 与 remem 专属文件。uninstall 只撤销 receipt 证明
   由 remem 写入的设置、block 与文件；用户改过的值冲突时停止并报告，不覆盖用户状态。
10. 激活前 closure-audit `src/observe/native.rs` 的输入面：remem-owned delivery files 必须按
    canonical path、receipt 与 marker 排除；其余 Claude topic files 若继续导入，须先经 redaction、
    source poisoning verdict、candidate review 与事务边界，错误必须向 hook 传播。只要仍存在
    direct-active insert、warning-only 继续或无法证明不自摄取，本 host/version 就是 no-go。

“把 Claude 原生目录直接作为 remem durable database”不采用：目录格式、加载上限与写入时机由
宿主管理，无法满足 remem 的事务、治理和跨宿主一致性。

### 3. Codex native-memory 发现与版本化解析

在 `ImportAction` 增加独立的 `CodexMemories` 子命令，对外命令固定为
`remem import codex-memories [--source <path>]`。默认源目录来自官方用户级位置，并允许测试/PoC
通过 `--source` 覆盖；路径解析集中在 host path 模块，不能散落读取 HOME。apply 必须携带先前
dry-run 返回的 `--expect-plan-digest <sha256>`；缺少或不匹配即拒绝写入。

该 source 不复用 backup/markdown 的 best-effort direct restore，也不复用 pack 的 clean-row
direct-active apply。pack planner/transaction 可作为结构参考，但分类闭集与落点必须强制为
`dedup`、`pending_review`、`quarantined` 或 `blocked`，不能包含 active `add`。

解析采用“先发现、再冻结计划、最后 apply”的两阶段模型：

```text
source dir (read-only)
  -> safe directory walk
  -> per-file stable read + metadata/content hash
  -> format detector (PoC-confirmed versions only)
  -> canonical external records
  -> dedup + trust/poisoning classification plan
  -> dry-run report OR one DB transaction
```

- 目录 walker 不跟随越界 symlink，只接受 PoC 已证明属于格式的文件；出现未知普通文件时错误，
  不以扩展名白名单静默跳过。
- stable read 在解析前后核对必要 metadata 与内容摘要；并发替换/截断导致本次失败。
- detector 返回明确 format id/version/fingerprint；没有可靠指纹时不猜测兼容格式。
- canonical record 只承载原生记忆文本、最小宿主 metadata、脱敏 source-relative id、内容摘要、
  格式版本和可验证的 workspace/repository evidence。宿主内容永远是数据，不解释其中的指令、
  路径、frontmatter 动作或 shell 文本。
- 一批任一文件/记录失败就不生成 apply plan；不会“导入已知、跳过未知”。
- detector 后、任何 DB/index 写入前调用现有 capture-adapter secret redaction/classification。检测到
  secret、redaction/classifier 失败时，plan 标为 blocked；不计算或持久化原文 hash，不创建 event、
  candidate 或 embedding，整批 apply 非零退出。

### 4. 幂等 provenance 与事务边界

对通过 secret boundary 的非敏感记录，幂等身份为
`sha256(format_id || format_version || canonical_redacted_content || destination_route)`，并携带固定
`source=codex_native`。`destination_route` 防止同一安全内容在 project 与 tool-owned review 间错误
折叠。源相对标识和脱敏内容摘要用于审计，不作为唯一身份，因 Codex 可重命名生成文件。

路由先消费宿主格式中 PoC 已验证的 workspace/repository evidence，并用现有 project resolver 映射
到 remem project/owner。证据缺失、矛盾或无法解析时，不使用 import cwd，也不猜 project；记录
进入 `owner_scope=tool`、`owner_key=codex-cli`、`context_class=search_only` 的 pending review。project
record 与 tool-owned record 的 identity namespace 分离，后续人工 re-scope 必须走现有 review/
provenance 审计，不直接改 active memory。

implementation 先搜索现有 captured-event provenance 与 candidate evidence 是否能表达该身份：

- 若现有字段/唯一键足够，复用它们并在同一事务中完成 event/provenance/candidate 写入；
- 若不足，先更新当前 `docs/specs/` 合同并添加最小 migration/唯一约束。不得用进程内 set 或
  扫描全文模拟持久化幂等。

apply 重新执行稳定读取和 planning，并先比较 `--expect-plan-digest`；只有与用户审查的 digest
精确相同才开启单一 DB transaction。每条 canonical record 进入 external-content 事件和
candidate 创建适配器，显式设置低信任来源并关闭该 source 的 auto-promote；投毒匹配进入
`quarantined`，其余进入 `pending_review`。不得调用 backup/markdown 的 active-memory 恢复路径。
事务提交后才打印成功；任何错误回滚全部记录和幂等标记。源目录始终只读。

### 5. Dry-run、诊断与 doctor

dry-run 与 apply 调用同一纯 planning 函数，差别只在是否打开写事务。机器可读输出至少包含：

- source state：`not_configured` / `ready` / `unreadable` / `unsupported_format`；
- detected format versions 与脱敏文件标识；
- `planned_import` / `dedup` / `quarantine` / `errors` 计数；
- plan digest；apply 必须以 `--expect-plan-digest` 绑定该值，源文件、路由、分类或配置变化导致
  digest 改变时拒绝提交并要求重新 dry-run。

若源目录不存在，CLI 以明确“无 Codex native memories”结果退出；不可读、非目录、未知格式、
不稳定读取或 DB 错误均非零退出并记录 error。doctor 使用相同 discovery/detector，但不读取或
打印完整正文，也不触发模型、下载或写入。

### 6. Codex hooks 覆盖审计

审计基线是当前 `src/install/hosts/codex.rs` 写入的官方 `hooks.json`。在隔离 Codex 会话中，对
官方支持的每个候选事件记录：是否触发、payload 字段、cwd/session/thread 标识、失败传播、超时
和与 Claude PreToolUse/PostToolUse observe 的语义差异。

同一矩阵分别从 core `remem install --target codex` 与 plugin
`activate-codex.js` 的显式 hooks-only 入口生成并比较；第三个 control 保持 plugin 只提供 MCP、未
激活 hooks。若两条激活入口的有效 `hooks.json`/feature state 或运行事件不同，先报告 drift，
不得用其中一条的证据代表另一条。

go 条件必须同时满足：关键 observe 输入可获得、事件顺序足以无重复地归属会话、失败可见、
不会把 secret 正文写入诊断、不会降低现有 SessionStart/Stop 捕获。否则结论为 no-go，并列出
缺失事件/字段和替代路线。本 issue 只提交审计证据；不得据此改 `build_hooks` 或安装事件集合。

### 7. 文档、版本与提交边界

- spec PR 仅包含 `specs/GH852/product.md` 与 `tech.md`，使用 `Refs #852`，保持 Draft。
- PoC evidence 须由 maintainer 确认可审计落点后另行补齐，不与 runtime code 混合，也不得以本
  spec 的计划文字冒充实测结果。
- product/tech 获 maintainer approval 且 issue 进入 `ready_to_implement` 后，才由
  `specrail-plan-tasks` 生成 `tasks.md`。
- implementation 若修改导入、安装、doctor、schema 或插件运行时，更新对应当前
  `docs/specs/`、README 和版本同步 surfaces，并按 repo version-sync skill 验证。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001`, `B-013` | isolated PoC harness/evidence | 真实 Claude/Codex 版本、基线/清理摘要与可复现记录人工 review |
| `B-002`–`B-004`, `B-016`, `B-019` | Claude settings resolver/installer + native-memory input/output | startup-window actual-content proof；marker preservation；single-block commit/recovery；manifest-based SessionStart exclusion；maintenance-window activation/rollback；no direct-active/self-ingest/warning-only path |
| `B-005`, `B-006` | Codex walker/detector/parser | source tree before/after hash 相同；unknown/malformed/partial/concurrent-write fixtures 全部 fail-closed |
| `B-007` | shared import planner | 同一 fixture 的 dry-run/apply plan digest 和分类一致；dry-run DB/source hash 不变 |
| `B-008`, `B-010` | provenance key + DB transaction | first apply/second apply counts；rename dedup；injected DB failure leaves zero partial rows |
| `B-009`, `B-014`, `B-018` | secret boundary + trust/poisoning/candidate adapter | safe input pending；injection quarantined；secret/classifier error leaves no event/candidate/hash/index；snapshot 无正文/secret |
| `B-011` | discovery + doctor | absent/empty/unreadable/not-directory/unsupported status table and exit-code tests |
| `B-012` | hooks audit evidence | core install、plugin hooks-only、plugin-only MCP 三态矩阵；与 Claude observe path 比较；go/no-go review |
| `B-015` | SpecRail gates | route/workflow checks reject implementation before approval/readiness |
| `B-017` | project resolver + tool-owned fallback route | verified project fixture 精确归属；ambiguous/conflicting evidence 只进 Codex search-only review；cwd 不影响结果 |

## 数据流

```text
Claude opt-in
  -> effective settings resolver
  -> native input closure audit (no direct-active / self-ingest / warning-only)
  -> dry-run conflict/ownership plan
  -> stage setting + receipt + prepared manifest (hook_only)
  -> atomic MEMORY.md delivery-block commit
       -> matching setting/manifest/block = native_active
       -> Claude startup loads actual block content
       -> SessionStart excludes exactly the block's manifest ids
       -> any mismatch = visible error, no successful SessionStart

Codex native files (read-only)
  -> stable discovery + version detector
  -> pre-persistence secret boundary (blocked batch on secret/error)
  -> project evidence resolver OR Codex tool-owned search-only route
  -> canonical external records
  -> provenance/dedup + poisoning classification
  -> dry-run report
       or single DB transaction
            -> captured external evidence
            -> pending_review / quarantined memory_candidates
            -X-> active memories

Codex official hooks
  -> isolated real sessions
  -> event/schema evidence matrix
  -> go/no-go document only
```

### 持久化与迁移

本 spec 不预先声明新表。implementation discovery 必须优先复用现有 event/candidate provenance；
只有无法以数据库约束实现 `B-008`/`B-010` 时才增加最小 migration。任何 migration 都需 schema
drift、upgrade、rollback 和重复运行测试，并在当前 DB spec 中登记。Codex 源文件不进入 install
receipt，也不被 remem 写回。

## 备选方案

- **直接扫描后写 active memories**：拒绝。绕过候选 review、投毒过滤和来源信任边界。
- **复用 backup/markdown import**：拒绝。两者的所有权和晋升语义与宿主生成的不可信内容不同。
- **只按路径记录已导入**：拒绝。生成文件可改名，且路径不能证明内容身份。
- **识别到部分文件就部分成功**：拒绝。用户无法知道遗漏内容，重试还会产生不透明混合状态。
- **双向同步 Codex/Claude 文件**：拒绝。宿主格式是生成状态，写回会制造冲突与升级风险。
- **默认启用 Claude redirect**：拒绝。改变宿主数据所有权，必须 opt-in 且可回滚。
- **扩大现有 Claude topic-file direct import**：拒绝。当前路径直接写 active memory 且错误仅
  warning；未接入统一 verdict/candidate/error contract 前必须 no-go。
- **再次“迁移到 hooks.json”**：拒绝。当前实现已经使用官方 hooks；本 issue 只做覆盖审计。
- **把缺失研究报告内容从 issue 反推补齐**：拒绝。无法追溯且会制造伪证据。

## 风险

- Security：宿主记忆包含任意文本、prompt injection 和 secret。解析器不执行内容；secret/
  classifier error 在任何正文/hash/event/candidate/index 持久化前阻断整批；安全内容仍强制低信任、
  禁 auto-promote，日志只显示脱敏标识。GH-855 的 source-evidence/runtime 防线在当前 main 仍是
  spec 而非可假设依赖；GH-852 implementation 必须等待其落地或独立证明同等闭环。相关实现需要
  mandatory human security review。
- Data integrity：Claude 双写或 Codex 部分提交会制造不可逆分叉。所有权 resolver、receipt、
  frozen plan 与 DB transaction 是合并前硬门禁。
- Compatibility：两个宿主都可改变生成格式/事件 schema。只支持 PoC 指纹，未知版本 fail-visible，
  不以宽松 serde/default 字段静默兼容。
- Privacy：PoC 与测试使用合成内容和隔离 HOME；不得把真实用户原生记忆提交到 fixture、日志或 PR。
- Performance：大目录读取要有后续 PoC 定出的文件/总字节上限；达到上限必须明确失败，不能截断
  后报告成功。doctor 不做全量正文输出。
- Maintenance：Claude 路径解析必须成为单一来源，避免 installer 与 runtime 再次漂移；Codex
  detector 按格式版本拆分并用真实脱敏 golden fixture 验证。

## 测试计划

- [ ] Claude isolated-HOME real PoC：默认目录、自定义目录、已有冲突值、多个 worktree、写入上限、
      uninstall/rollback 与清理证明。
- [ ] Codex isolated-HOME real PoC：真实格式、空目录、未知版本、并发写入、hooks event matrix 与
      清理证明。
- [ ] parser golden tests：每个已支持格式的脱敏真实 fixture，加 malformed/non-UTF8/symlink/
      oversized/unknown fixtures。
- [ ] import focused tests：dry-run no-write、first/second apply、rename dedup、poison quarantine、
      plan-digest mismatch、project/tool routing、secret/classifier blocked no-persistence、active-memory
      unchanged、injected transaction failure rollback、source tree hash unchanged。
- [ ] Claude install focused tests：scope rejection、path normalization、unknown-key preservation、
      marker 外内容保留、PoC-confirmed startup window 实际加载与容量不足、manifest exclusion、active-session
      refusal、每次文件替换后的 crash/restart recovery、inconsistent state 阻止 SessionStart、
      reverse commit rollback、conflict no-overwrite、无双开/双关；native topic input 不直写 active、
      remem-owned file 不自摄取、错误不 warning-only。
- [ ] doctor focused tests：四态输出、退出状态和 secret redaction。
- [ ] `cargo test import` 及实际匹配到的 import module tests。
- [ ] Claude/Codex install、context native-memory、candidate/poisoning 与 doctor focused tests。
- [ ] `cargo fmt --check`。
- [ ] `cargo check`。
- [ ] `cargo clippy --all-targets -- -D warnings`。
- [ ] `cargo test`。
- [ ] `python3 scripts/ci/check_plugin_version_sync.py`（若修改版本/runtime surfaces）。
- [ ] `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`。
- [ ] 当前 spec 阶段：`python3 checks/check_workflow.py --repo .` 与 `write_spec` route gate。
- [ ] spec approval 后生成真实 `tasks.md`，再运行
      `python3 checks/check_workflow.py --repo . --spec-dir specs/GH852`；当前已确认它只因
      `missing tasks.md` 失败，不得以占位任务绕过。
- [ ] `git diff --check origin/main...HEAD`，并人工审查无源文件改写、secret 输出或治理绕过。

## 回滚方案

spec 未批准时关闭 Draft PR，不产生运行时影响。implementation 未合并时关闭 implementation PR。

若 Claude 接管已合并：使用 receipt 校验当前值仍由 remem 拥有，原子恢复备份设置，停止新目录
写入，并验证旧/新目录中只剩一个 remem 权威输出；遇到用户修改冲突时停止并报告，不覆盖。

若 Codex import 已合并：禁用/回滚新的 CLI source 与 detector，不触碰宿主源文件。已创建候选按
`source=codex_native` provenance 审计；未人工批准的候选可事务性 discard。已由人工批准晋升的
记忆不得自动删除，须由单独审计和人工决定。schema migration 如有，只能按其独立 rollback plan
处理，不能用 destructive reset。

hooks 审计本身无运行时回滚。任何后续 hooks 变更必须另走 issue/spec/implementation 与人工门禁。

本文件不构成 `spec_approval`。报告核对、产品/技术批准和 `ready_to_implement` 完成后，才可生成
`tasks.md` 并开始实现。
