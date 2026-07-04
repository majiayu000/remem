# Product Spec

## Linked Issue

GH-720

## User Problem

个人 agent 行为数据（Claude Code / Codex 会话 transcript）目前被 remem 和 refine 两套系统各自摄入、各建一套 schema。remem 只在 Stop hook 时摄入活跃会话，不批量扫描、不回填历史；refine 有批量扫描和 12 维 facet 提取，但存储与 remem 割裂。后果：

- 下游工具（recap、better、cognitive-portrait、codex-retrospective）各写各的 jsonl 解析器，或分别依赖两个库
- 同一份 transcript 被摄入两遍，语义无法互查（raw 消息查不到 facet，facet 追不回原文）
- remem 缺少"管理我所有 AI 会话记忆"的旗舰场景，产品叙事不完整

终态：remem 是所有 agent 会话数据的唯一存储与查询面，refine 逐步归档。分三阶段推进，每阶段独立可停。

## Goals

- Phase 1：remem 获得批量、增量、幂等的会话目录摄入能力（`remem ingest-sessions`），历史回填从"设计上不做"变为"可选"
- Phase 1：raw 查询面支持时间窗口（since/until）、窗口内会话列表、每会话消息采样——覆盖 recap 类工具的全部数据需求
- Phase 2：refine 停止自行摄入，facet 提取改读 remem——双轨摄入终结
- Phase 3：facet 提取成为 remem 的一种后台 job，产出落 remem 新表；refine 归档

## Non-Goals

- 不改变 curated memory（`memories` 表）的语义、review 流程与 SPEC-raw-archive 确立的 raw/curated 分层
- 不在本 spec 内实现消费方（recap/better/cognitive-portrait）的迁移，消费方各自开 issue
- 不做 refine `items/documents` → remem 的数据库级迁移（历史用重摄入覆盖）
- Phase 2 的中间态（remem 存储 + refine 分析）是稳定形态；Phase 3 可独立推迟，不构成技术债

## Behavior Invariants

### Phase 1 — 批量摄入

1. `remem ingest-sessions` 扫描 `~/.claude/projects/*/*.jsonl` 与 `~/.codex/sessions/**/*.jsonl`，将消息摄入 `raw_messages`；对同一数据集重复执行第二次，新增行数为 0（幂等）。
2. 摄入是增量的：自上次运行以来 mtime/size 未变化的文件被跳过，不重新解析。
3. 扫描根目录可配置并可追加（默认上述两处；可加入如 `~/.claude/remote-sessions/<host>/projects` 的同步目录），每条摄入记录保留来源根的标识。
4. 单个文件解析失败（损坏行、编码问题）只记录该文件失败，不中断整批；失败文件在下次运行时重试。
5. 历史回填默认全量，支持 `--since <date>` 下界；回填不影响 Stop hook 的实时摄入路径，两者并发运行不产生重复行。
6. 摄入结束输出机器可读汇总：扫描文件数、跳过数、新增消息数、失败数。

### Phase 1 — 时间窗口查询

7. raw 查询接受 `since`/`until`（epoch 或 ISO8601），只返回窗口内消息；不给窗口时行为与现状完全一致（向后兼容）。
8. 新增"窗口内会话列表"查询：返回每个会话的 session id、project、来源根、窗口内首末消息时间、消息数；支持按 project 过滤。
9. 会话列表支持每会话采样前 N 条用户消息（N 可配置），用于 recap 类摘要场景。
10. 以上查询在 CLI（`remem raw`）与 MCP raw 工具两个面均可用，输出结构一致。

### Phase 2 — refine 降级为消费者

11. refine 的 facet 提取以 remem 的 raw 查询输出为唯一输入源；切换后 refine 自身的目录扫描代码路径被移除或永久停用。
12. 切换后对同一时间窗口，refine 产出的 facet 数量与切换前基线的差异有可解释的对账报告（允许差异，但每类差异必须归因）。

### Phase 3 — facet 提取内化

13. facet 提取作为 remem 后台 job 运行，产出落独立 `facets` 表，每条 facet 可追溯到来源会话与消息区间；不复用/不扭曲 `observations` 表。
14. facet 提取的 LLM 用量记入 `ai_usage_events`；历史回填范围有硬上限（默认最近 90 天，可配置），超出范围不自动消费 LLM。
15. facet 可按维度 + 时间窗口 + project 查询，供 cognitive-portrait 类消费方使用。

## Acceptance Criteria

- [ ] Phase 1 两个不变量组（1-6、7-10）各有对应的自动化测试与一次真实数据人工验证（对照 recap 脚本在相同窗口的会话计数）
- [ ] Phase 1 完成后 recap 可完全基于 remem 查询重写（不再直接解析 jsonl），作为验收用例
- [ ] Phase 2 有对账报告，差异全部归因
- [ ] Phase 3 的 facets 表设计、job 接入、成本上限在实现前有独立 tech 审查
- [ ] 每阶段收尾更新本 spec 的状态记录

## Edge Cases

- 活跃会话文件正在被 Claude Code 追加写入时执行批量摄入：只摄入已完整落盘的行，半行截断不报为文件失败
- 同一会话被 hook 实时摄入过、又被批量扫描扫到：UNIQUE 约束去重，计数汇总中体现为跳过而非新增
- 跨机器同步目录中出现与本机相同 project 名：来源根标识必须能区分两者
- 时钟偏移/时区：窗口过滤一律基于 transcript 内的 UTC 时间戳，不用文件 mtime 做语义过滤（mtime 仅用于增量跳过）
- 超大 transcript（数百 MB）：流式逐行解析，内存占用有界
- refine 切换期（Phase 2）双方短暂并行：以 remem 为准，refine 旧库只读留档

## Rollout Notes

- Phase 1 纯增量（新子命令 + 查询参数），无默认行为变化，无迁移风险
- Phase 2 需要 refine 侧一次切换发布，切换前跑对账；可随时切回旧路径（代码停用而非立删）
- Phase 3 落地后，portfolio 中 refine 标记 `merge-into:remem`，refine README 顶部加归档指引
- 每阶段完成时在 GH-720 留状态评论，作为阶段门禁记录
