# Agent 时代记忆库优化方向调研（2026-07）

Status: Research reference
Date: 2026-07-16
Method: 4 路并行网络调研（学术 / 开源竞品 / 宿主平台 / 评测基准）+ 代码库现状核对
Truth baseline: **origin/main**（注意：初稿曾基于落后 522 commits 的分支得出错误差距，本文已按 main 修正，见 §6 勘误）

---

## 1. TL;DR

- **"单 repo 自动捕获记忆"已被四大宿主原生化**（Claude Code auto memory 默认开、Codex Memories、Copilot Agentic Memory 默认开、Cursor Memories），remem 靠"自动捕获"本身已无差异化。
- 长期价值在宿主结构性缺位的四件事：**跨宿主统一、跨机器同步、可审计本地化、git 原生团队记忆**。
- main 上仍成立的技术差距（本次开 issue 的范围）：**cross-encoder rerank、写入侧 contextual enrichment、graph_edges 检索接入、宿主原生记忆数据源化、注入"少而精"预算、capture 路径投毒防护、团队 pack review 工作流**。
- 明确不做：HyDE、重型图数据库（Neo4j 路线）、实时 LLM 巩固、继续单独强化"单 repo 自动捕获"叙事。

## 2. 外部环境

### 2.1 宿主平台（威胁最直接）

| 宿主 | 原生记忆现状（2026-07） |
|---|---|
| Claude Code | auto memory 默认开启，`~/.claude/projects/<p>/memory/` + MEMORY.md 索引；"Auto Dream" 后台整理；`autoMemoryDirectory` 可重定向；subagent 独立 memory；`InstructionsLoaded` hook 可审计注入 |
| OpenAI Codex | 原生 Memories（`~/.codex/memories/`，两阶段抽取+consolidation 子代理）全 surface 滚动中；EEA 默认关；v0.144.0（2026-07-09）落地 hooks.json |
| GitHub Copilot | Agentic Memory 默认开（Pro/Pro+），repo 级、跨 agent 共享、28 天过期——唯一平台化团队记忆的宿主 |
| Cursor | Memories per-project；官方承认不跨项目、不跨成员 |
| Anthropic API | memory tool（client-side 文件型）+ context editing；官方数据 +39% / token -84%；官方还上架了 Remember 插件 |

宿主互不相通（各自目录/云、格式各异）是结构性空隙。MCP 2026-07-28 RC 转向无状态（sampling/roots 弃用），协议不会做官方 memory 原语——对第三方 memory server 是利好，但依赖 sampling 的设计需在 12 个月内迁移。

### 2.2 竞品

- **claude-mem**（87K★，Node+Chroma，hook→LLM 压缩→SQLite→注入）：同生态位头部，已扩展到 Codex/Gemini/Copilot 多宿主。remem 可守差异：Rust 单二进制、离线本地、Codex 深度。
- Mem0（61K★）/ Zep-Graphiti（29K★）/ Letta（24K★）/ Cognee（28K★）：重平台路线，与本地插件层重叠低。学界实测 Mem0/Zep 的 FactConsolidation 仅 18%/7%（arXiv:2606.01435）——冲突消解是全行业公开弱点，确定性规则优于 LLM 判 freshness。
- 新玩家：EverOS（Markdown+SQLite+LanceDB hybrid，2026-06）、ReMe、memsearch。Beads（任务图记忆，25K★）是互补品。

### 2.3 学术共识（可抄的机制）

- Mem0 四操作（ADD/UPDATE/DELETE/NOOP）；Zep bitemporal 软失效（remem 已有 memory_facts 雏形）。
- **ReasoningBank（arXiv:2509.25140）：检索注入 k=1 最优（49.7%），k=4 反降到 44.4%**——"注入更多记忆常常有害"，失败轨迹须蒸馏成 guardrail。
- HippoRAG 2：图检索可纯算法化（PPR），LLM 只花在构建期。
- Sleep-time compute（Letta）：离线巩固摊薄成本——remem autodream 方向正确。
- 记忆安全：MINJA（arXiv:2503.03704）证明普通交互即可投毒（>95% 成功率）；持久记忆=持久攻击面。
- 反向结论：HyDE 多基准不敌普通 dense；agent 自带多轮 grep 时重型图 DB 边际收益存疑（arXiv:2604.09666）。

### 2.4 评测基准

- LoCoMo 已被审计出 6.4% 答案键损坏（理论满分 ~93.6%），厂商数字互相打架；LongMemEval judge 偏松；Letta 用纯文件系统+grep 拿 74% 是全赛道警钟——**必须先打赢 grep 基线和全上下文基线**。
- coding-agent 跨 session 记忆基准接近空白：SWE-ContextBench（轨迹复用，oracle 摘要 +8pp）、Stompy（记忆不提升代码质量但省 15-28% 探索成本）是仅有的两个。remem 的 `src/eval/coding_bench`（#385）方向与此吻合；对外发布已被 #384 止损，可按新证据复核但不属本轮范围。
- 检索改进通行收益（通用检索文献外推，memory 语料噪声更大需打折）：hybrid RRF Recall@5 0.695 vs BM25 0.644；两阶段 rerank 0.816（再 +17.4%）；Anthropic contextual retrieval 检索失败率 -49%（叠加 rerank -67%）。

## 3. main 现状核对（事实）

已落地（勿重复立项）：
- 真语义 embedding：`local-onnx`(fastembed) 为默认 Cargo feature，`src/retrieval/embedding/local_semantic.rs`，多模型向量键 + backfill + eval gate（#682/#714-#716/#729/#731）。
- 4 通道 RRF 检索（README §Search Architecture）；temporal 检索通道（#481）；bitemporal memory_facts（M4 #381）。
- Export + git 可提交 memory pack + provenance-aware 导入（#678，`src/cli/actions/pack_import.rs`）。
- 投毒防护（candidate review / pack import 层，`src/api/tests/candidate_review_poisoning.rs`）。
- coding-agent A/B benchmark 骨架（`src/eval/coding_bench/`，#385 contract）。
- autodream、procedural memory、lifecycle add/update/invalidate/noop。

仍成立的差距：
1. **rerank 缺失**——仅 test 文件提及；随 M5 #383（NOT_PLANNED）一起被搁置。
2. **写入侧 contextual enrichment（retrieval_text）无实现**——此前决策"P0（真 embedding）就位前 defer"；P0 已就位，解除前提。
3. **graph_edges 契约完备但检索/注入零消费**（docs/graph-contract.md 自述；`git grep graph_edges origin/main -- src/retrieval src/context` 为空）。
4. **宿主原生记忆零集成**——无 autoMemoryDirectory 接管、无 Codex `~/.codex/memories/` 导入、Codex hooks.json 未评估。
5. **注入预算未按 k-少而精收紧**（ReasoningBank 证据出现在 remem 现有设计之后）。
6. **capture/extraction 路径的指令性文本过滤待核**（现有防护在 candidate review / pack import 层）。
7. **团队 pack 缺多人 review/merge 工作流**（#678 只做了单机 export/import；Copilot 平台化团队记忆是新证据）。
8. LoCoMo v1 multi-hop 39.0% / v2 temporal 40.5% 仍是弱分项（informational only）。

## 4. 优化方向（对应 GitHub issues）

| 优先级 | 方向 | 核心证据 |
|---|---|---|
| P0 | 检索二阶段：本地 cross-encoder rerank | 两阶段 Recall@5 +17.4%；复活 #383 的 rerank split 部分 |
| P0 | 写入侧 contextual enrichment（retrieval_text 索引字段） | Anthropic -49% 检索失败率；LLM extraction 管线已在，边际成本低 |
| P0 | 宿主原生记忆数据源化（Claude autoMemoryDirectory / Codex memories / Codex hooks.json） | "接管而非对抗"；宿主原生化后第三方唯一高杠杆位 |
| P1 | graph_edges 接入检索（entity 一跳/两跳 + 轻量 PPR） | HippoRAG 2；multi-hop 39% 弱分项；已投入未变现 |
| P1 | SessionStart 注入"少而精"（预算收紧 + k 扫描 eval） | ReasoningBank k=1 最优；官方 200 行/25KB 索引模式同向 |
| P1 | capture 路径投毒防护补全 | MINJA；hook 自动捕获面大于对话面 |
| P2 | 团队记忆 review/merge 工作流（基于 #678 pack） | Copilot 团队记忆默认开；"数据留在自己 git、记忆走 PR 评审"无人占位 |

不建议投入：HyDE、Neo4j 级图数据库、实时 LLM 巩固、query rewrite（观望）。

## 5. 风险与复核项

- Anthropic Remember 插件与 Letta Code 类"记忆内置 agent"可能压缩第三方插件空间（置信度中）。
- #384（公开 benchmark 发布）与 team memory 均为此前主动止损的决策；本文提供的新外部证据（Copilot 团队记忆、benchmark 空白）可作为复核输入，但复核决定权在 maintainer，不在本轮 issue 范围内自动重启。
- 收益数字均为通用检索文献外推（置信度中），落地前应过 golden eval gate 验证。

## 6. 勘误

初稿（会话内报告）基于 `codex/plugin-version-sync` 分支（落后 origin/main 522 commits），错误声称："vector 是 feature-hash 假向量、无 export、无投毒防护、coding benchmark 空白"。以上在 main 均已落地，详见 §3。教训：代码现状盘点必须先 `git fetch` 并以 origin/main 为基线。

## 7. 主要来源

学术：Mem0 arXiv:2504.19413 · A-Mem 2502.12110 · HippoRAG 2 2502.14802 · ReasoningBank 2509.25140 · Zep 2501.13956 · MIRIX 2507.07957 · Sleep-time 2504.13171 · Memory-R1 (ACL 2026) · MINJA 2503.03704 · FactConsolidation 2606.01435 · Do-We-Still-Need-GraphRAG 2604.09666 · SWE-ContextBench 2602.08316
平台：code.claude.com/docs/en/memory · learn.chatgpt.com/docs/customization/memories · GitHub changelog（Copilot Agentic Memory 2026-01-15 / 默认开 2026-03-04）· Anthropic memory tool / context editing 文档 · MCP 2026-07-28 RC
竞品：claude-mem / mem0 / graphiti / cognee / letta / supermemory / EverOS / Beads（GitHub API star 数为 2026-07-16 实测）
评测：LongMemEval arXiv:2410.10813 · LoCoMo 损坏审计（Penfield Labs）· Zep/Mem0 争议（getzep/zep-papers#5）· MemoryArena 2602.16313 · hybrid+rerank 数据 2604.01733
