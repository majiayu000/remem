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

## 目标

- 用隔离的真实 Claude Code 环境验证 `autoMemoryDirectory` 的读取、写入、容量和生命周期行为，
  并确定 remem 与 Claude 原生记忆唯一、无双写的所有权规则。
- 提供 `remem import --source codex-memories` 的单向、幂等、可 dry-run 导入，把可识别的 Codex
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
- 不在项目级 `.claude/settings*.json` 注入 `autoMemoryDirectory`；该设置只允许用户级、策略级或
  显式 `--settings` 范围。
- 不自动合并、覆盖或删除用户手工维护的 Claude/Codex 文件。
- 不在研究报告缺失时补写其结论或把 issue 摘要当作报告全文。

## Behavior Invariants

1. `B-001`：三个交付面均须 PoC 先行。PoC 记录必须包含宿主版本、隔离目录、输入步骤、观察到
   的文件/事件、退出状态和清理结果；推断或模拟输出不能替代真实 Claude Code/Codex 证据。
2. `B-002`：Claude `autoMemoryDirectory` 只可在官方允许的用户/策略/显式 settings 范围配置，
   使用绝对路径或 `~/` 路径；不得写项目/本地 settings，也不得越出用户明确选择的目录。
3. `B-003`：Claude 接管必须有单一写入者规则。启用后，remem 现有
   `~/.claude/projects/<slug>/memory/` 同步不得继续向旧目录生成第二份状态；停用或回滚后不得遗留
   两个均被视为权威的目录。
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
   Codex 源文件与内容摘要，但诊断不得打印文件正文或 secret。
10. `B-010`：导入一批记录要么完整写入对应的 provenance、事件与候选计划，要么全部回滚。
    DB 写入、解析或投毒分类失败不得留下“已导入”标记或半条候选。
11. `B-011`：源目录不存在属于明确的“未配置/无原生记忆”状态；路径存在但不可读、不是目录或
    格式不识别属于错误。CLI 与 doctor 必须区分这些状态并给出可行动原因。
12. `B-012`：Codex hooks 审计以 remem 当前官方 `hooks.json` 安装为基线，逐项记录 SessionStart、
    Stop 及可用工具事件的输入字段、触发时机、失败语义和 observe 等价性；结论只能是带缺口清单
    的 go 或 no-go，不得通过本 issue 静默改变捕获链路。
13. `B-013`：PoC 使用隔离 HOME/数据目录，开始前记录基线，结束后恢复设置并证明没有修改真实
    用户宿主记忆。任何必须触碰真实宿主配置的步骤都需单独人工确认。
14. `B-014`：导入和 doctor 输出不得泄露 memory 正文、凭据、token、绝对路径中的敏感用户名
    或宿主生成的隐私内容；详细诊断使用可安全展示的相对标识或摘要。
15. `B-015`：研究报告合入并核对、产品/技术 spec 人工批准、`ready_to_implement`、最终 review、
    merge 与 release 均保留为 human gates。

## 验收标准

- [ ] 缺失研究报告已进入可审计提交，且 spec 中引用的结论与报告一致；否则 spec 不批准。
- [ ] Claude PoC 在隔离环境中记录真实版本与读写证据，并确定启用、重复启用、停用和失败回滚
      下的唯一目录所有权规则。
- [ ] `autoMemoryDirectory` dry-run 不改配置；apply 原子保留未知设置；旧的 remem native-memory
      同步不会形成第二写入面。
- [ ] Codex PoC 记录至少一个真实已识别格式、一个未知/畸形格式和目录缺失/不可读状态；不从
      单一 fixture 推断全部宿主版本。
- [ ] Codex import 的 dry-run 与 apply 分类一致；重复 apply 不增加重复事件/candidate；任一解析
      或写入错误均整批回滚。
- [ ] 导入结果带 `source=codex_native`、源摘要和格式版本，且只能进入 `pending_review` 或
      `quarantined`，投毒样本不能直接成为 active memory。
- [ ] doctor 能区分未配置、可用、不可读和不支持格式，且不输出记忆正文或 secret。
- [ ] Codex hooks 审计使用真实会话事件，明确与 Claude observe 粒度的覆盖差异及 go/no-go，且
      本 issue 不改变 hooks 运行时。
- [ ] focused tests、格式/编译检查、完整测试和真实 PoC 清理检查通过。
- [ ] spec PR 与 implementation PR 分离；本 spec 只有 product/tech，不含 tasks 或运行时代码。

## 边界情况

- Claude 设置已有用户自定义 `autoMemoryDirectory`：不得覆盖；dry-run 报告冲突，apply 要求显式
  所有权决策并保留可回滚值。
- Claude MEMORY.md 或主题文件已存在：不得盲目拼接或截断；PoC 必须验证官方加载上限与 remem
  生成内容的去重边界。
- 同一 Claude 项目有多个 worktree：不得因路径归一化让不同项目误共享或让同项目重复注入。
- Codex 目录为空、包含子目录、临时文件、符号链接、超大文件、非 UTF-8、并发写入或在扫描后
  被替换：必须遵守已验证格式与一致性边界，未知项不能静默忽略。
- 多个 Codex 文件表达同一事实或同一文件被改名：内容身份去重，来源证据仍保留所有可验证
  关联。
- 源内容含 prompt injection、伪造 frontmatter、路径穿越文本或 secret：按不可信内容处理，
  禁止把正文解释为命令、路径或配置。
- 数据库在 apply 中失败：事务回滚，源文件不变，下次运行仍可重试。
- 宿主升级改变格式或事件 schema：版本指纹不匹配即 fail-visible；先补 PoC/spec，再扩适配器。

## 发布说明

若后续 implementation 获批，发布说明必须明确这是 opt-in 的宿主原生记忆接入：Claude 配置
可回滚，Codex 导入只读且需 review，hooks 仅产生审计结论。不得把实验格式描述为长期稳定 API。

官方参考：

- [Claude Code memory and `autoMemoryDirectory`](https://code.claude.com/docs/en/memory)
- [Claude Code data and privacy boundaries](https://code.claude.com/docs/en/claude-directory)
- [Codex local memory](https://developers.openai.com/codex/config-advanced/#memory)
- [Codex hooks](https://developers.openai.com/codex/hooks/)

本文件只定义产品契约，不构成 `spec_approval` 或实现授权。
