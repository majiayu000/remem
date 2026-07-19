# Product Spec

## Linked Issue

GH-852

## 用户问题

Claude Code 与 Codex 都开始维护宿主原生的本地记忆，但这些状态与 remem 的治理、去重和审计
链路彼此隔离。用户因此可能同时得到 remem 注入和宿主原生记忆，形成重复、冲突或来源不明的
上下文；也无法安全地把既有 Codex 记忆纳入 remem。

本 issue 引用的研究报告
`docs/research/agent-memory-optimization-research-2026-07.md` 当前不在 `origin/main`。在报告以可审计
提交进入仓库并完成核对前，它不能作为已验证需求依据；这是本 spec 的批准阻塞项，不影响先把
issue 与官方宿主文档已经支持的行为写成可审查契约。

此外，issue 中“把 Codex 捕获从 wrapper 迁移到 hooks.json”的前提已经过时：remem 当前正式
安装路径已经写入 `~/.codex/hooks.json`。本 issue 只审计官方事件面能否补齐 Codex observe 级
捕获，并产出 go/no-go 结论，不实施第二次迁移。

当前 main 还存在一个必须显式纳入安全门禁的事实：Claude PostToolUse 的 Write/Edit observe 路径
会读取 `.claude/projects/*/memory/*.md`（不含 `MEMORY.md`）并直接写入 active memory，失败只记录
warning。`autoMemoryDirectory` 若扩大该读取面，不能沿用这一直接晋升和 warning-only 语义。

## 目标

- 用隔离的真实 Claude Code 环境验证 `autoMemoryDirectory` 的读取、写入、容量和生命周期行为，
  并以 remem 数据库为唯一权威，确定 native bridge 与 SessionStart 互斥交付、无重复注入的规则。
- 提供 `remem import codex-memories` 的单向、幂等、可 dry-run 导入，把可识别的 Codex
  本地记忆作为不可信外部内容送入现有 candidate review 与投毒过滤，而不是直接晋升。
- 审计 remem 当前 Codex `hooks.json` 事件覆盖度，重点比较 Claude 侧 observe 级捕获，形成带真实
  事件证据的 go/no-go 结论。
- 对目录缺失、权限错误、未知格式、部分解析和宿主版本漂移提供明确诊断，不静默丢数据。
- 保留宿主原生源文件；PoC、安装和导入均不得自动删除或改写用户的原生记忆。

## 非目标

- 不把 Claude 与 Codex 原生记忆当作新的权威 durable-memory 存储，也不做双向同步。
- 不在本 issue 中迁移、扩展或替换 Codex hooks；审计结论若要求运行时变更，另开 issue/spec。
- 不推断、承诺或硬编码当前未由真实 PoC 确认的 Codex 生成文件格式。
- 不绕过 `memory_candidates`、人工 review、来源信任分级或投毒隔离。
- 不把当前 Claude native-memory topic-file 的直接 active-memory 导入扩大到自定义目录；该路径
  未接入与新来源相同的 redaction、source verdict、candidate review 和 error-visible 契约前，
  native bridge 必须保持 no-go。
- 不在项目级 `.claude/settings*.json` 注入 `autoMemoryDirectory`；该设置只允许用户级、策略级或
  显式 `--settings` 范围。
- 除用户显式 opt-in 后维护 remem 专属文件及 `MEMORY.md` 中的 marker-bounded block 外，不合并、
  覆盖或删除用户手工维护的 Claude/Codex 内容。
- 不在研究报告缺失时补写其结论或把 issue 摘要当作报告全文。

## Behavior Invariants

1. `B-001`：三个交付面均须 PoC 先行。PoC 记录必须包含宿主版本、隔离目录、输入步骤、观察到
   的文件/事件、退出状态和清理结果；推断或模拟输出不能替代真实 Claude Code/Codex 证据。本次
   spec recovery 不触碰真实用户目录，也不把未执行的 PoC 写成已完成证据。
2. `B-002`：Claude `autoMemoryDirectory` 只可在官方允许的用户/策略/显式 settings 范围配置，
   使用绝对路径或 `~/` 路径；不得写项目/本地 settings，也不得越出用户明确选择的目录。
3. `B-003`：remem 数据库始终是唯一权威；Claude 目录只是可重建的交付 cache。接管启用后，
   remem 只拥有 `remem_sessions.md` 和 `MEMORY.md` 内一个 marker-bounded 交付块，不拥有 Claude
   生成的其余 MEMORY/topic 内容。只有真实位于官方 startup 加载窗口内的交付块正文才算 native
   delivery；topic-file 链接不算。旧目录不得继续生成第二份 remem 输出。
4. `B-004`：Claude 配置变更必须支持 dry-run，变更前备份，保留未知键，以原子方式写入；失败
   时恢复原配置与目录所有权状态，并输出 error 级可定位诊断。
5. `B-005`：Codex 导入是只读、单向操作。它不得修改、移动或删除 `~/.codex/memories/` 下的
   文件，也不得依赖手工编辑 Codex 生成状态。
6. `B-006`：Codex 解析器只接受 PoC 已识别且版本化的格式。目录存在但文件未知、记录畸形、
   文件读取不完整或一批中部分记录无法解析时，整次 apply 必须失败且不产生部分提交；报告须
   指明文件和原因，不能把失败项记为普通 skip。
7. `B-007`：`--dry-run` 与 apply 使用同一发现、解析、去重和安全分类计划；dry-run 不写 remem
   数据库和源目录，并明确列出 planned import、dedup、quarantine 与 error 数量。
8. `B-008`：同一来源记录重复运行导入不会创建重复事件或 candidate。来源路径不是稳定身份；
   幂等键至少绑定规范化内容摘要、已验证格式版本和 `source=codex_native` provenance。
9. `B-009`：所有 Codex 原生内容按外部/低信任输入处理，进入既有候选与投毒链路；新候选默认
   `pending_review` 或 `quarantined`，不得自动晋升到 active memory。provenance 必须能追溯到
   Codex 源文件与脱敏内容摘要，但诊断不得打印文件正文或 secret。
10. `B-010`：导入一批记录要么完整写入对应的 provenance、事件与候选计划，要么全部回滚。
    DB 写入、解析或投毒分类失败不得留下“已导入”标记或半条候选。
11. `B-011`：源目录不存在属于明确的“未配置/无原生记忆”状态；路径存在但不可读、不是目录或
    格式不识别属于错误。CLI 与 doctor 必须区分这些状态并给出可行动原因。
12. `B-012`：Codex hooks 审计以 remem 当前官方 `hooks.json` 安装为基线，逐项记录 SessionStart、
    Stop 及可用工具事件的输入字段、触发时机、失败语义和 observe 等价性；结论只能是带缺口清单
    的 go 或 no-go，不得通过本 issue 静默改变捕获链路。审计必须覆盖 core installer 与 Codex
    plugin 的显式 hooks-only 激活，并证明两条入口生成相同基线；插件仅加载 MCP、未激活 hooks
    的状态不得被误报为已捕获。
13. `B-013`：PoC 使用隔离 HOME/数据目录，开始前记录基线，结束后恢复设置并证明没有修改真实
    用户宿主记忆。任何必须触碰真实宿主配置的步骤都需单独人工确认。
14. `B-014`：导入和 doctor 输出不得泄露 memory 正文、凭据、token、绝对路径中的敏感用户名
    或宿主生成的隐私内容；详细诊断使用可安全展示的相对标识或摘要。
15. `B-015`：遵守 SpecRail v0.2.1。当前 `ready_to_spec` 只授权 `write_spec`；研究报告合入并
    核对、产品/技术 spec 人工批准、maintainer 设置 `ready_to_implement`、security decision、
    最终 PR review、merge 与 release 均保留为 human gates。在这些门禁完成前不得生成
    `tasks.md`、修改 runtime 或声称实现已获授权。
16. `B-016`：native bridge 激活时，当前激活 manifest 中已由 Claude 原生 memory 交付的 remem
    条目必须从 SessionStart 注入集合排除；未激活、manifest 不完整/过期或回滚后只能走
    SessionStart。不得在一个 SessionStart 中同时交付同一 stable memory id/content hash，也不得
    因去重状态错误同时关闭两条路径。安装/回滚必须在无活动 Claude 会话的 maintenance window
    完成；无法证明该条件时拒绝切换。有效 setting、prepared manifest 与 startup-window marker
    generation/digest 必须全部匹配；任何不一致都要 error 并阻止成功的 SessionStart，不能猜测
    fallback。若 PoC 证明宿主 hook 失败不能阻止会话，则 native bridge 结论必须为 no-go。
17. `B-017`：Codex record 只能依据宿主提供且可验证的 workspace/repository evidence 绑定 remem
    project。无法可靠归属的 record 必须进入 `owner_scope=tool`、`owner_key=codex-cli`、
    `context_class=search_only` 的全局待审队列，绝不能把 import 命令的当前 cwd 当作来源项目。
18. `B-018`：任何 record 在写 event、candidate、embedding 或索引前必须通过既有 secret-redaction
    边界。检测到 secret、redaction 失败或分类器出错时，整批 apply 失败且不持久化正文、正文摘要、
    candidate 或“已导入”标记；dry-run 只报告脱敏文件标识和 `secret_blocked` 计数。
19. `B-019`：Claude native-memory 的输入与输出必须有独立、可证明的所有权边界。现有
    topic-file observe 直接写 active memory、warning-only 失败或任何 source-taint 缺口仍存在时，
    `autoMemoryDirectory` 接管不得激活。若后续保留该输入能力，它必须先复用已批准的统一
    redaction/poisoning verdict、candidate review、事务和 error-visible 契约；remem 自己生成的
    delivery files 还必须被明确排除，避免自摄取循环。

## 验收标准

- [ ] 缺失研究报告已进入可审计提交，且 spec 中引用的结论与报告一致；否则 spec 不批准。
- [ ] Claude PoC 在隔离环境中记录真实版本与读写证据，并确定启用、重复启用、停用和失败回滚
      下的唯一目录所有权规则。
- [ ] `autoMemoryDirectory` dry-run 不改配置；apply 原子保留未知设置；旧的 remem native-memory
      同步不会形成第二写入面；startup-window 交付块、prepared manifest 与 SessionStart exclusion
      使用同一 stable id/hash 集合，激活前后均无重复或遗漏。
- [ ] Codex PoC 记录至少一个真实已识别格式、一个未知/畸形格式和目录缺失/不可读状态；不从
      单一 fixture 推断全部宿主版本。
- [ ] Codex import 的 dry-run 与 apply 分类一致；重复 apply 不增加重复事件/candidate；任一解析
      或写入错误均整批回滚。
- [ ] 导入结果带 `source=codex_native`、源摘要和格式版本，且只能进入 `pending_review` 或
      `quarantined`，投毒样本不能直接成为 active memory。
- [ ] 有可靠项目证据的 record 只进入对应 project；无可靠证据的 record 只进入 Codex tool-owned
      search-only review，跨项目 fixture 不发生污染。
- [ ] secret、redaction failure 和 classifier failure fixtures 整批失败，DB 与索引中没有原文、
      原文 hash、candidate、embedding 或导入标记。
- [ ] doctor 能区分未配置、可用、不可读和不支持格式，且不输出记忆正文或 secret。
- [ ] Codex hooks 审计使用真实会话事件，明确与 Claude observe 粒度的覆盖差异及 go/no-go，且
      本 issue 不改变 hooks 运行时；core installer、plugin hooks-only 激活和 plugin-only MCP
      三种状态均被区分。
- [ ] Claude native-memory 输入路径已通过 closure audit：不得把 remem 生成文件摄取回 active
      memory；直接 active-memory 晋升、warning-only 失败或未统一 source verdict 任一仍存在时，
      `autoMemoryDirectory` 激活测试必须得到 no-go。
- [ ] focused tests、格式/编译检查、完整测试和真实 PoC 清理检查通过。
- [ ] spec PR 与 implementation PR 分离；本 spec 只有 product/tech，不含 tasks 或运行时代码。

## 边界情况

- Claude 设置已有用户自定义 `autoMemoryDirectory`：不得覆盖；dry-run 报告冲突，apply 要求显式
  所有权决策并保留可回滚值。
- Claude MEMORY.md 或主题文件已存在：不得盲目拼接或截断；PoC 必须验证官方加载上限与 remem
  生成内容的去重边界。如果不移动/截断既有 startup-loaded 用户内容就没有足够窗口容纳 remem
  交付块，native bridge 必须保持关闭并继续只用 SessionStart。
- Claude 在接管目录内写 topic file，或 remem 自己写 `remem_sessions.md`：输入 watcher 必须根据
  receipt、canonical path 与文件 ownership 明确区分；无法证明不会自摄取或直接晋升时停止激活。
- 同一 Claude 项目有多个 worktree：不得因路径归一化让不同项目误共享或让同项目重复注入。
- Codex 目录为空、包含子目录、临时文件、符号链接、超大文件、非 UTF-8、并发写入或在扫描后
  被替换：必须遵守已验证格式与一致性边界，未知项不能静默忽略。
- 多个 Codex 文件表达同一事实或同一文件被改名：内容身份去重，来源证据仍保留所有可验证
  关联。
- 源内容含 prompt injection、伪造 frontmatter、路径穿越文本或 secret：按不可信内容处理，
  禁止把正文解释为命令、路径或配置。
- 数据库在 apply 中失败：事务回滚，源文件不变，下次运行仍可重试。
- 宿主升级改变格式或事件 schema：版本指纹不匹配即 fail-visible；先补 PoC/spec，再扩适配器。
- Codex plugin 已加载但未显式激活 hooks：MCP 可用不等于 SessionStart/Stop 自动捕获已启用；
  status、doctor 与审计必须报告真实状态。

## 发布说明

若后续 implementation 获批，发布说明必须明确这是 opt-in 的宿主原生记忆接入：Claude 配置
可回滚，Codex 导入只读且需 review，hooks 仅产生审计结论。不得把实验格式描述为长期稳定 API。

官方参考：

- [Claude Code memory and `autoMemoryDirectory`](https://code.claude.com/docs/en/memory)
- [Claude Code data and privacy boundaries](https://code.claude.com/docs/en/claude-directory)
- [Codex local memory](https://learn.chatgpt.com/docs/customization/memories)
- [Codex hooks](https://learn.chatgpt.com/docs/hooks)

本文件只定义产品契约，不构成 `spec_approval` 或实现授权。
