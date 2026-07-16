# Product Spec

## Linked Issue

GH-851（Epic: GH-849）

## 用户问题

remem 当前把全文、实体、事实、向量等候选通道按查询条件组合后，通过 RRF 产生最终顺序。
这种一阶段融合能够扩大召回，但不能直接判断“查询与一条候选记忆整体上是否真正相关”。对于改写表达、
隐含关联和表面词汇相近但语义不相关的候选，前几名仍可能排序不理想，进而同时影响显式
`remem search` 和 SessionStart 自动注入的记忆质量。

用户需要一个完全在本地运行的二阶段 cross-encoder：只重排已经通过现有项目、分支、owner、
抑制和陈旧性规则的 RRF top-N 候选，再返回 top-k。该能力不能让 hook 因联网下载模型而卡住，
也不能在模型缺失、损坏或推理失败时静默伪装成“已启用”。

## 目标

- 在现有 RRF 之后增加可关闭的本地 cross-encoder rerank，将合格候选从 top-N 重排为 top-k。
- `remem search` 的标准记忆检索与 SessionStart 隐式检索使用同一个 rerank 行为合同和同一实现路径。
- 模型只通过显式安装动作下载并校验；search、API/MCP 和 hook 运行时不得发起模型下载或其他
  网络请求。
- 模型关闭、缺失、损坏、超时、取消或推理失败时，完整保留现有 RRF 结果，并通过稳定的
  `disabled_reason`、error 级日志和 doctor/status 诊断暴露真实状态。
- 用仓库本地 golden eval 的同条件 A/B 结果判断质量，不引用论文指标；MRR@10 与 Hit@5 不劣化，
  并以预先登记的相关 slice 指标达到至少 5 个绝对百分点提升作为启用目标。
- 建立可审计的 rerank 分阶段 timing 和 SessionStart p95 门禁，只有满足人工批准的预算后才允许
  默认启用。

## 非目标

- 不替换现有候选通道、RRF、过滤、置信度、source-anchor demotion 或分页语义。
- 不让 reranker 召回新记忆，也不允许它把项目外、owner 不匹配、被抑制或已被过滤的记忆重新加入。
- 不改变原始 transcript 搜索、无查询 list 路径或 multi-hop 搜索；首期范围是标准 curated-memory
  search 和 SessionStart 的隐式查询路径。
- 不使用远程 rerank 服务，不在 hook/search/API/MCP 请求内下载模型，不把查询或记忆发送到网络。
- 不在本 spec 中选定具体模型、模型版本、top-N/top-k 数值、输入长度或数字化 p95 预算；这些值
  必须来自本地证据并由 human approval 冻结。
- 不声称论文、模型卡或外部 benchmark 的提升数字等同于 remem 的本地质量收益。
- 不在本 spec PR 中写实现任务、修改运行时代码、下载模型或改变默认配置。

## Behavior Invariants

1. `B-001`：启用且本地模型完整可用时，标准 `remem search` 与 SessionStart 都必须先完成现有
   候选生成、RRF 和全部资格过滤，再把同一规范化查询及按 RRF 排序的最多 top-N 个合格候选交给
   同一个 rerank stage；最终结果只能来自该候选集合，并按 cross-encoder 分数取 top-k。
2. `B-002`：top-N 必须大于或等于 top-k。候选少于 top-k 时只重排实际候选；候选为空时返回空，
   不加载模型、不报伪错误。相同查询、候选内容、模型 manifest、配置和数据库快照必须得到稳定顺序；
   同分时依次按原 RRF rank 和稳定 memory id 打破平局。
3. `B-003`：rerank 不得绕过或改变项目、分支、owner、scope、suppression、staleness、置信度及
   其他现有资格规则。它不能添加 top-N 之外的候选，也不能因模型输入截断而把原始记忆内容写回或
   改写数据库。
4. `B-004`：模型文件必须完全本地化，并由显式、用户发起的下载动作安装到 remem 管理的模型目录；
   下载完成前必须校验声明的文件集合、字节数和哈希并原子发布 manifest。SessionStart、search、
   API/MCP 和其他查询路径在任何状态下都不得调用网络下载。
5. `B-005`：显式关闭 rerank 时，结果顺序和分页必须与变更前的 RRF 基线一致，诊断状态为关闭且
   `disabled_reason` 稳定可见。显式关闭不是 doctor 失败。
6. `B-006`：配置为启用但模型缺失、manifest 缺失、文件损坏或哈希不匹配时，请求不得下载模型，
   不得使用未校验文件，也不得返回部分 rerank 顺序；它必须原子回退到完整 RRF 顺序，同时在查询
   诊断、SessionStart 诊断、error 级日志和 doctor/status 中暴露稳定的 `disabled_reason`。doctor
   对已启用但缺失或损坏的模型必须失败。
7. `B-007`：推理初始化失败、单批推理失败、超过批准的请求预算或请求被取消时，不得发布部分重排
   结果。若调用方仍接受结果，必须返回完整 RRF 顺序和相应 `disabled_reason`；若调用方取消了整个
   请求，则按现有取消协议终止。错误不得降为 warning 后静默继续。
8. `B-008`：并发 search、API/MCP 与 SessionStart 请求可共享已加载的只读模型资产，但不得共享
   可变请求状态。模型初始化必须至多一次成功发布；失败初始化不能暴露半初始化实例，单个请求的
   取消、超时或推理错误不能污染其他请求的候选和结果。
9. `B-009`：分页所需窗口超出人工批准的 top-N 上限时，不得只重排不足的前缀后继续分页，从而
   产生重复或跳项。实现必须对该请求完整使用基线 RRF 顺序并暴露
   `disabled_reason=pagination_window_exceeded`，或由后续 human-approved contract 明确扩大窗口；
   不得隐式改变 top-N。
10. `B-010`：标准 search 和 SessionStart 必须使用同一个候选文档投影、模型调用、排序、失败回退、
    `disabled_reason` 枚举及 timing 定义。二者可以保留各自的一阶段候选生成和权限/项目过滤，但不
    得复制两份 rerank 算法或产生不同的模型输入语义。
11. `B-011`：查询诊断必须公开 rerank 是否 requested、是否 applied、模型标识/manifest 哈希、
    输入候选数、输出数、top-N/top-k、`disabled_reason` 以及加载、推理和总耗时。SessionStart 的
    运行诊断和 error 日志必须携带同一状态；不得把诊断文本伪装成记忆正文注入模型。
12. `B-012`：启用候选必须使用同一提交的 `eval/golden.json`、同一数据库 fixture、同一机器与
    运行配置，对 rerank-off 和 rerank-on 做本地 A/B。paraphrase 与 associative 两个 slice 各自的
    MRR@10 和 Hit@5 均不得低于 off 基线；两个 slice 合并后的 MRR@10 和 Hit@5 也不得降低，且
    预先登记的一个合并主指标必须提升至少 `0.05` 绝对值。不得在看到结果后更换主指标、slice、
    top-N/top-k 或模型以规避失败。
13. `B-013`：rerank 必须分别记录模型加载、推理和 rerank 总耗时，并纳入 SessionStart 总耗时
    统计。默认启用前必须在 human-approved 的参考机器、样本量、冷/热启动策略和数字化 p95 预算
    下通过；冷启动与热启动不得混在一个 p95 中。当前 `main` 没有权威的 SessionStart 数字化 p95
    硬门槛，因此该预算缺失是 spec approval/enablement 阻塞项，不能自行填入经验数字。
14. `B-014`：离线状态下，已完整安装并校验的模型必须可正常 rerank；未安装或损坏的模型按
    `B-006` fail-visible 回退。离线本身不得触发自动下载、无限重试或改变数据库。
15. `B-015`：禁用开关必须在不删除模型和不迁移数据库的情况下恢复基线 RRF。模型升级、质量门禁
    失败、p95 超预算或线上错误率异常时，维护者可以关闭 rerank；关闭后 search 与 SessionStart
    都必须走相同的基线行为，旧模型文件不得影响结果。
16. `B-016`：该能力在 exact model preset/version/hash、top-N/top-k、候选输入上限、参考性能
    profile、数字化 SessionStart p95 预算和本地 A/B 证据全部由 maintainer 批准前保持 draft/off。
    `ready_to_implement`、最终 review、merge、release 和默认启用均保留为 human gates。

## 验收标准

- [ ] 标准 `remem search`（含其 API/MCP service 调用方）和 SessionStart 在资格过滤后调用同一
      rerank stage，且 top-N 之外的记忆永不进入结果。
- [ ] 显式关闭时，固定 fixture 上结果、顺序、分页和 `has_more` 与现有 RRF 基线一致。
- [ ] 本地模型通过显式命令下载并验证 manifest；网络被禁用时，hook/search 不尝试下载，已安装
      模型仍可工作。
- [ ] 缺失、损坏、hash mismatch、初始化失败、推理失败、超时、取消和分页窗口超限均有测试；
      没有部分排序，回退时返回完整 RRF 顺序并给出稳定 `disabled_reason`。
- [ ] doctor/status 能区分显式关闭、可用、缺失和损坏；已启用但缺失/损坏返回失败。
- [ ] search explain/诊断和 SessionStart timing/error 证据包含相同的 rerank 状态、模型标识、
      候选计数、原因和分阶段耗时。
- [ ] 本地 golden A/B 证明 paraphrase/associative 的 MRR@10 与 Hit@5 均不劣化，预登记合并主指标
      提升至少 0.05 绝对值；证据记录 commit、dataset hash、manifest hash、配置和原始报告。
- [ ] 人工批准数字化 SessionStart p95 预算及 benchmark profile 后，冷/热启动各自通过；批准前
      rerank 不得默认启用。
- [ ] `cargo fmt --check`、`cargo check`、focused tests、`cargo test` 和完整 PR preflight 通过。
- [ ] spec PR 仅 `Refs #851` / `Refs #849`；implementation 必须等待 spec approval 和
      `ready_to_implement`，最终 merge/release 仍需人工授权。

## 边界情况

- top-N 为 0、top-k 为 0、N 小于 k、候选不足、重复候选和相同 rerank 分数：配置非法则启动/请求
  fail-visible；合法空/不足候选按 `B-002` 处理，重复候选不得产生重复结果。
- 查询为空或当前路径没有可用于 rerank 的规范化查询：保持该路径现有行为并以稳定 reason 表示
  rerank 未应用，不把空字符串送入模型。
- 候选正文为空、超长或包含非 ASCII：使用同一 UTF-8 安全、确定性的候选投影和截断规则；截断
  只影响模型输入，不改数据库或展示内容。
- offset/limit 使所需分页窗口超出 top-N：按 `B-009` 完整回退，不能混合部分 rerank 与 RRF。
- 下载中进程退出、磁盘写满或 hash 校验失败：旧的已验证版本继续可用；新版本不得以半成品发布。
- 多进程 hook 与服务同时加载或升级模型：运行时只读已原子发布的 manifest 版本；显式下载不得
  让其他进程观察到部分文件。
- 请求在模型加载前、批次之间或推理返回后取消：不发布部分结果，不留下数据库写入或下载任务。
- 模型损坏但仍能被 ONNX 打开：manifest/file hash 校验仍先于推理，不能仅凭加载成功判定完整。
- 权限与隐私：reranker 输入只包含请求已有权看到的本地候选；不新增网络发送或跨项目读取。
- accessibility：该变更没有图形交互；CLI/诊断使用结构化状态和文本原因，不以颜色作为唯一信号。

## 发布说明

这是一个默认保持关闭、无需数据库迁移的本地检索能力。模型是显式安装的可删除资产；回滚只需
关闭 rerank 即可恢复现有 RRF 行为。发布说明必须列出所选模型 preset/version/hash、磁盘需求、
显式下载/状态/doctor 命令、top-N/top-k、质量 A/B 证据和批准后的性能预算，并明确 hook 不会下载
模型。

当前 `main` 缺少数字化 SessionStart p95 硬门槛、已批准的模型与本地 rerank A/B 基线；这些是
批准与默认启用阻塞项，不得由外部论文数字补足。本文件仅为 draft，`spec_approval`、
`ready_to_implement`、最终 PR review、merge 和 release 均保留为 human gates。
