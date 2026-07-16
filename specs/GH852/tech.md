# Tech Spec

## Linked Issue

GH-852

## Product Spec

[`product.md`](product.md)

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Import CLI | `src/cli/archive_types.rs`, `src/cli/actions/import.rs`, `src/cli/tests.rs` | `ImportAction` 只有 backup/markdown；backup 可直接恢复 durable 数据 | Codex 原生记忆需要独立 source，不能复用直接晋升语义 |
| Candidate governance | `src/memory_candidate/`, `src/memory/poisoning.rs`, `docs/ARCHITECTURE.md` | 候选带来源信任、review/quarantine 与投毒模式；durable facts 不应直接写 active memories | 导入必须接到候选治理边界且禁用 auto-promote |
| Claude native memory | `src/context/claude_memory/paths.rs`, `runtime.rs`, `render.rs` | remem 向硬编码 `~/.claude/projects/<slug>/memory/remem_sessions.md` 单向同步 | `autoMemoryDirectory` 接管可能形成新旧双写面 |
| Claude install | `src/install/hosts/claude.rs`, `src/install/paths.rs`, install receipt/doctor modules | 安装会合并用户设置并记录可卸载状态，但没有 `autoMemoryDirectory` | PoC 后的 opt-in 配置必须沿用原子变更与回滚契约 |
| Codex install/hooks | `src/install/hosts/codex.rs`, `src/install/config.rs`, `README.md` | 已正式写 `~/.codex/hooks.json`；Codex 当前主要安装 SessionStart/Stop，Claude 才有 observe 级工具 hooks | 第三交付物是覆盖审计，不是 wrapper 迁移 |
| Host paths/doctor | `src/install/paths.rs`, `src/doctor/` | 没有 Codex native memories path 或格式状态 | 需要可覆盖路径发现和未配置/错误/不支持诊断 |
| Specs | `docs/specs/memory-poisoning-defense/`, `docs/specs/README.md` | 现行投毒与 candidate 契约已存在 | implementation 要更新当前行为契约，不复制历史 spec |

issue 引用的 `docs/research/agent-memory-optimization-research-2026-07.md` 不在当前 main。其进入
可审计提交并与本设计核对，是 spec approval 的前置条件。

## 设计方案

### 1. PoC 先行与证据包

implementation 前先在隔离 HOME 与 `REMEM_DATA_DIR` 下执行两个宿主 PoC，并把结果放入当前
spec packet 的后续人工批准提交（具体证据文件名在 PoC 阶段 search-first 后确定，避免预造重复
文档）。证据包括：

- 宿主二进制版本和设置来源；
- 初始目录树的路径脱敏清单与内容摘要；
- 可复现命令/交互步骤、退出码、事件名称和 schema 摘要；
- 写入前后摘要、并发/异常行为和清理后摘要；
- “观察事实”与“设计推断”分栏，以及不支持版本清单。

PoC 不读取或复制真实用户记忆；若宿主无法在隔离 HOME 工作，停止并请求新的人工授权，不把
真实配置当测试夹具。

### 2. Claude `autoMemoryDirectory` 所有权模型

PoC 验证官方设置约束后，在现有 Claude host installer 中增加显式 opt-in，而不是默认安装：

1. 读取用户级/策略级/显式 settings 的有效值和来源；项目/local scope 一律拒绝。
2. 规范化绝对路径或 `~/` 路径，但不解析用户目录外的符号链接为可写目标。
3. 若已有非 remem 值，dry-run 报告 conflict；apply 不覆盖，除非未来 CLI 设计提供单独的显式
   adopt 动作并再次人工批准。
4. 使用现有设置合并、临时文件原子替换、备份和 install receipt 机制；保留所有未知 JSON 键。
5. 接管启用时，`sync_to_claude_memory` 必须从同一解析后的 effective directory 写入 remem 专属
   文件，或被明确禁用；不得继续向硬编码旧目录写第二份。remem 只拥有自己的文件，不拥有
   Claude 的 MEMORY.md/主题文件。
6. uninstall/rollback 只撤销 receipt 证明由 remem 写入的设置与文件；用户后续改过的值发生
   冲突时 fail-visible，不覆盖用户状态。

“把 Claude 原生目录直接作为 remem durable database”不采用：目录格式、加载上限与写入时机由
宿主管理，无法满足 remem 的事务、治理和跨宿主一致性。

### 3. Codex native-memory 发现与版本化解析

在 `ImportAction` 增加独立的 `CodexMemories` variant，对外命令为
`remem import --source codex-memories`。默认源目录来自官方用户级位置，并允许测试/PoC 通过
显式 path 参数覆盖；路径解析集中在 host path 模块，不能散落读取 HOME。

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
- canonical record 只承载原生记忆文本、最小宿主 metadata、脱敏 source-relative id、内容摘要与
  格式版本。宿主内容永远是数据，不解释其中的指令、路径、frontmatter 动作或 shell 文本。
- 一批任一文件/记录失败就不生成 apply plan；不会“导入已知、跳过未知”。

### 4. 幂等 provenance 与事务边界

幂等身份为 `sha256(format_id || format_version || canonical_content)`，并携带固定
`source=codex_native`。源相对标识和原始内容摘要用于审计，不作为唯一身份，因 Codex 可重命名
生成文件。

implementation 先搜索现有 captured-event provenance 与 candidate evidence 是否能表达该身份：

- 若现有字段/唯一键足够，复用它们并在同一事务中完成 event/provenance/candidate 写入；
- 若不足，先更新当前 `docs/specs/` 合同并添加最小 migration/唯一约束。不得用进程内 set 或
  扫描全文模拟持久化幂等。

apply 按冻结 plan 开启单一 DB transaction。每条 canonical record 进入 external-content 事件和
candidate 创建适配器，显式设置低信任来源并关闭该 source 的 auto-promote；投毒匹配进入
`quarantined`，其余进入 `pending_review`。不得调用 backup/markdown 的 active-memory 恢复路径。
事务提交后才打印成功；任何错误回滚全部记录和幂等标记。源目录始终只读。

### 5. Dry-run、诊断与 doctor

dry-run 与 apply 调用同一纯 planning 函数，差别只在是否打开写事务。机器可读输出至少包含：

- source state：`not_configured` / `ready` / `unreadable` / `unsupported_format`；
- detected format versions 与脱敏文件标识；
- `planned_import` / `dedup` / `quarantine` / `errors` 计数；
- plan digest，使 apply 可与刚审查的 dry-run 结果对照。

若源目录不存在，CLI 以明确“无 Codex native memories”结果退出；不可读、非目录、未知格式、
不稳定读取或 DB 错误均非零退出并记录 error。doctor 使用相同 discovery/detector，但不读取或
打印完整正文，也不触发模型、下载或写入。

### 6. Codex hooks 覆盖审计

审计基线是当前 `src/install/hosts/codex.rs` 写入的官方 `hooks.json`。在隔离 Codex 会话中，对
官方支持的每个候选事件记录：是否触发、payload 字段、cwd/session/thread 标识、失败传播、超时
和与 Claude PreToolUse/PostToolUse observe 的语义差异。

go 条件必须同时满足：关键 observe 输入可获得、事件顺序足以无重复地归属会话、失败可见、
不会把 secret 正文写入诊断、不会降低现有 SessionStart/Stop 捕获。否则结论为 no-go，并列出
缺失事件/字段和替代路线。本 issue 只提交审计证据；不得据此改 `build_hooks` 或安装事件集合。

### 7. 文档、版本与提交边界

- spec PR 仅包含 `specs/GH852/product.md` 与 `tech.md`，使用 `Refs #852` / `Refs #849`，保持 Draft。
- PoC evidence 可在同一 spec PR 的后续批准提交中补齐，但不混入 runtime code。
- product/tech 获 maintainer approval 且 issue 进入 `ready_to_implement` 后，才由
  `specrail-plan-tasks` 生成 `tasks.md`。
- implementation 若修改导入、安装、doctor、schema 或插件运行时，更新对应当前
  `docs/specs/`、README 和版本同步 surfaces，并按 repo version-sync skill 验证。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001`, `B-013` | isolated PoC harness/evidence | 真实 Claude/Codex 版本、基线/清理摘要与可复现记录人工 review |
| `B-002`–`B-004` | Claude settings resolver/installer + native-memory path | scope/path table tests；dry-run no-write；unknown-key preservation；atomic failure/rollback tests |
| `B-005`, `B-006` | Codex walker/detector/parser | source tree before/after hash 相同；unknown/malformed/partial/concurrent-write fixtures 全部 fail-closed |
| `B-007` | shared import planner | 同一 fixture 的 dry-run/apply plan digest 和分类一致；dry-run DB/source hash 不变 |
| `B-008`, `B-010` | provenance key + DB transaction | first apply/second apply counts；rename dedup；injected DB failure leaves zero partial rows |
| `B-009`, `B-014` | trust/poisoning/candidate adapter + renderer | safe input pending；injection quarantined；active memory count unchanged；snapshot 无正文/secret |
| `B-011` | discovery + doctor | absent/empty/unreadable/not-directory/unsupported status table and exit-code tests |
| `B-012` | hooks audit evidence | official-event matrix compared with current SessionStart/Stop and Claude observe path；go/no-go review |
| `B-015` | SpecRail gates | route/workflow checks reject implementation before approval/readiness |

## 数据流

```text
Claude opt-in
  -> effective settings resolver
  -> dry-run conflict/ownership plan
  -> atomic settings + receipt
  -> exactly one remem-owned native-memory output path

Codex native files (read-only)
  -> stable discovery + version detector
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
- **再次“迁移到 hooks.json”**：拒绝。当前实现已经使用官方 hooks；本 issue 只做覆盖审计。
- **把缺失研究报告内容从 issue 反推补齐**：拒绝。无法追溯且会制造伪证据。

## 风险

- Security：宿主记忆包含任意文本、prompt injection 和 secret。解析器不执行内容；候选强制低
  信任/禁 auto-promote；日志只显示摘要。相关实现需要 mandatory human security review。
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
      active-memory unchanged、injected transaction failure rollback、source tree hash unchanged。
- [ ] Claude install focused tests：scope rejection、path normalization、unknown-key preservation、
      conflict no-overwrite、atomic write failure、receipt-aware rollback、single-writer path。
- [ ] doctor focused tests：四态输出、退出状态和 secret redaction。
- [ ] `cargo test import` 及实际匹配到的 import module tests。
- [ ] Claude/Codex install、context native-memory、candidate/poisoning 与 doctor focused tests。
- [ ] `cargo fmt --check`。
- [ ] `cargo check`。
- [ ] `cargo clippy --all-targets -- -D warnings`。
- [ ] `cargo test`。
- [ ] `python3 scripts/ci/check_plugin_version_sync.py`（若修改版本/runtime surfaces）。
- [ ] `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`。
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
