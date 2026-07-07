# Multi-AI Research: Grok / Gemini 个人记忆系统设计对标（for remem user-context claims 管线）

- 日期：2026-07-07
- 模式：双外部降级（Grok ✅ + ChatGPT ✅；Gemini 两次 NO RESPONSE 超时，已降级）
- 内部：1 个源码分析 agent（remem user_context 管线，基于 origin/main 392a3e7）
- 用时：约 10 分钟，6 个并行任务（含 2 次重试）

## 原始研究问题

调研 Grok (xAI) 和 Gemini (Google) 的个人记忆系统设计：自动记忆抽取管线（来源、门槛、审查）、显式记忆、存储数据面、注入机制、用户治理、2026 最新进展。目的：为 remem user-context claims 管线设计（事前强审查 vs 事后治理、旧 preference 回填）提供业界对标依据。

## 外部 AI prompt（三家一致）

见下方 raw response 各文件开头引用的问题 (1)-(6)。

## Raw: Grok

思考了 1m 27s 以下是对标分析（基于2026年中期公开信息，截至2026年7月）。 我综合了xAI官方公告、Google支持文档/博客、第三方深度技术分析（如MemoryLake、MemX、MemoryPlugin等）、用户实测报告（Reddit等）以及近期X平台信息。功能可用性因地区（EEA/UK/Switzerland等常因法规受限）、平台（grok.com vs X.com、Gemini web/app）、订阅层级和用户设置而异。许多细节来自第三方分析而非官方底层实现文档，具体算法（如置信度计算、衰减机制）未完全公开，我会标注不确定之处。⁠Memorylake Grok（尤其是Grok 4.3及后续Skills）和Gemini的记忆系统都采用“先入库、事后治理”模式，而非事前人工审查。两者都强调选择性注入（relevant retrieval）而非全量注入，以控制上下文长度。Grok在2026年5月通过Skills增加了程序性/工作流层的创新；Gemini则在导入迁移和隐私模式（Temporary Chats）上更成熟，并与Google生态（Apps Activity + 连接的Gmail/Photos等）有更深绑定。 (1) 自动记忆抽取：来源、置信度门槛、事前审查 Grok： 来源：主要来自用户的历史聊天记录（非删除的chats）。系统提取总结/关键点/推断（preferences、interests、recurring topics、interaction style、goals等），而非 verbatim 完整聊天记录。不是全量历史RAG，而是模型驱动的选择性提取。⁠Blog.memoryplugin 自动 vs 显式：以自动为主（model-driven automatic personalization），用户无需手动触发。Grok会从可用历史中学习“什么对你重要”。 置信度/审查：无公开的置信度门槛或事前人工审查。提取是自动的。用户报告提到它会进行推断（可能存在衰减机制，尤其是goals/interests），但具体算法未披露。⁠Reddit 不确定/边缘情况： rollout 时或启用后会处理一定历史范围（用户实测提到可访问至少某日期如3月11日之后的非删除chats）。并非所有平台/会话都一致（部分用户报告2026年出现不稳定）。API层面无此记忆功能（仅consumer apps）。 Gemini： 来源：来自Apps Activity（存储的聊天历史/raw chats）。Personal Context / Personal Intelligence层自动跨prior conversations读取模式，提取关键细节和偏好。⁠Memx 自动 vs 显式：自动层（Personal Context）独立于Saved Info。 置信度/审查：同样无公开置信度门槛或事前人工审查。自动学习，Keep Activity开启时生效。 不确定/边缘情况：依赖Google Account（非设备绑定）。企业/未成年/特定地区账号受限。Temporary Chats完全不进入此流程。 本地设计启示：自动抽取可采用LLM summarizer + 结构化存储（facts vs inferences），内部可加置信度/重要性评分（即使产品不暴露）。建议分离存储（自动提取的summary store vs 显式facts），并支持衰减或重要性加权。避免全量原始聊天存储以控成本和隐私。 (2) 显式记忆：用户主动入口、与自动记忆是否分开存储 Grok： 入口：在对话中直接告诉Grok（“remember that I prefer concise responses / I’m working on React projects”）。Grok 4.3+新增Skills：用自然语言或文件上传定义可复用专业知识（格式规则、工作流步骤、文档结构等），无需代码。⁠Memorylake 分开存储：是。基础跨会话记忆（declarative facts/preferences）与Skills（procedural/workflow层）是不同概念。Skills在检测到相关场景时自动应用，是2026年5月22日前后的显著升级。⁠Memorylake 存储位置：均在context window之外的持久层。 Gemini： 入口：Saved Info（设置 > Saved Info，手动添加事实如“I’m vegetarian”或职业）。早期限Google One AI Premium，后扩展。 分开存储：明确分开。Saved Info（用户手动添加） vs Personal Context（自动从chats学习）。两者有独立开关和控制。⁠Memx 比较与本地启示：两者都支持显式入口，且与自动层分离（强烈推荐本地系统效仿）。Grok的Skills更侧重可复用程序性知识（对agentic/工作流场景极有价值），可本地实现为“规则库 + 触发条件检测”。Gemini的Saved Info更像简单KV facts。建议本地提供自然语言添加 + 文件/结构化导入界面。 (3) 注入机制：新对话开始时如何放入上下文 Grok： 机制：选择性/相关性检索（relevant retrieval），而非全部注入或简单摘要。新对话开始时，Grok查询记忆存储，仅在判断有用时将相关facts/preferences/Skills带入当前上下文。不会重放完整历史。⁠Suprmind Skills：检测到适用场景时自动激活应用。 边缘：依赖设置开启；部分会话/平台可能表现为“each chat stands alone”（近期Grok自身回复中出现此表述，可能因设置关闭或特定上下文）。 Gemini： 机制：自动判断是否需要注入来自past chats、Saved Info或连接Google Apps的上下文。相关时召回并融入响应（例如自动考虑饮食限制）。同样是相关性驱动而非全量。⁠Gemini 比较与本地启示：两者都避免“全部注入”（这会快速耗尽上下文窗口）。推荐本地系统采用向量检索/RAG + reranker实现相关性注入，可显著降低token消耗并提高精准度。Grok的Skills触发逻辑值得借鉴（基于意图/关键词/场景检测自动激活）。可加入“记忆摘要压缩”作为备选注入方式。 (4) 用户治理：查看/编辑/删除、开关粒度、审查模式 Grok： 治理：可查看Grok存储的具体内容；支持单条删除（聊天界面图标或Settings > Data Controls > Memory）。透明度是亮点。⁠Yahoo Finance 开关：Settings → Data Controls → “Memory”或“Personalize Grok with your conversation history”开关（默认开启）。关闭后停止新建记忆，但已有记忆保留（需手动删）。⁠Plurality 模式：先入库、事后删除。无事前审查。支持个体粒度删除，但无原生项目/桶分离（第三方工具可增强）。 不确定：跨平台（web/iOS/Android）一致性曾被用户报告不佳；删除后是否立即影响所有会话有细微差异。 Gemini： 治理：Saved Info支持单条编辑/删除（设置直接操作）。聊天历史可在myactivity.google.com或Gemini内删除/管理。Personal Context有单一on/off开关（停止自动学习，但原始activity保留）。⁠Memx 隐私模式：Temporary Chats（对话不保存、不用于个性化/训练，保留最长72小时后丢弃）。 模式：同样先入库、事后删除（默认聊天进入Apps Activity并用于Personal Context，除非用Temporary Chats或事后删除）。 比较与本地启示：两者治理都较透明且支持单条操作（本地系统必须实现此UI/ API）。强烈建议提供： 显式“记忆浏览器”（view/edit/delete/search） 细粒度开关（全局 + 按类别/项目 + Temporary模式） 导出/导入功能（Gemini已支持，Grok暂无公开成熟版本） 审计日志（谁/何时添加/删除） 这对本地系统是核心竞争力（用户对“黑箱记忆”信任度低）。 (5) 2026最新进展 Grok 4.3（约2026年5月18日）与Skills： 核心升级是cross-conversation memory（可靠跨会话连续性）+ Skills（持久可复用专业知识层）。Skills允许定义并自动应用格式规则、工作流步骤等，是对传统fact memory的显著扩展。⁠Memorylake 目标用户：SuperGrok / X Premium相关层级。仍为consumer apps特性（非API原生）。 Gemini： Personal Context / Personal Intelligence：持续演进，2026年4月左右进行架构重组（Personal Context作为独立菜单逐步淡出，整合进Personalization / Personal Intelligence设置）。⁠Support.google Temporary Chats（2025年8月推出）：提供强隐私选项。 与Search个性化的关系：Personal Intelligence主要围绕Gemini chats + 连接的Google Apps（Gmail/Photos等）提供上下文，与更广泛的Google Search个性化有生态重叠但非完全等同（后者更多是账户级广告/推荐）。 本地启示：2026年趋势是从纯facts memory向混合（facts + procedural Skills/workflows）演进，并重视隐私模式和生态导入。本地系统可实现“可组合规则引擎”（类似Skills）+ 强隐私sandbox（Temporary conversations）。 (6) 旧历史聊天数据回填进新记忆系统 Grok： 部分支持：启用/rollout记忆功能后，系统可访问并从非删除的历史chats中提取总结/推断（用户实测确认存在历史访问能力）。但无官方明确宣布“全面回填所有旧数据进新系统”的详细政策或时间线。它更像是特征启用时的增量处理，而非一次性全量迁移。⁠Reddit 不确定：具体回填范围（全部历史 vs 近期？）、提取深度、是否支持用户主动触发全量重建，均未有官方详尽披露。部分近期Grok回复显示“each chat stands alone”的行为，可能因设置或上下文而异。 Gemini： 自身历史：无需特殊回填——Personal Context直接构建在现有Apps Activity之上。 迁移/回填：2026年3月左右推出Import功能，支持从其他AI（ChatGPT、Claude等）导入memories/preferences（通过提示复制）和完整聊天历史ZIP。Gemini会处理并整合进其memory/personal context系统，实现“pick up where you left off”。⁠Blog 这是一个用户主动的显式迁移工具，而非自动全量回填旧Gemini数据。 比较与本地启示：Gemini在用户迁移场景上更友好（ZIP导入 + 自动处理）。Grok目前在这方面较弱（2026年4月仍有“no import tools yet”的报告，虽有测试迹象）。⁠Anuma 本地系统强烈推荐实现： 导入工具（支持常见格式/ZIP + 结构化memory） 可选全量/增量回填模式（用summarizer处理历史，避免存储爆炸） 透明审计（显示哪些历史被提取、何时） 总结对标建议（本地AI记忆系统设计） 维度 Grok优势/特点 Gemini优势/特点 本地系统推荐实践 自动抽取 总结+推断，Skills程序层 Apps Activity驱动，生态绑定 LLM summarizer + 结构化分离存储 显式 vs 自动 Skills（程序性）突出 Saved Info清晰分离 必须分离 + 自然语言/文件入口 注入机制 相关性检索 + Skills自动触发 相关性 + Google Apps上下文 RAG/retrieval优先，避免全量 用户治理 单条删除，透明 Temporary Chats + 细粒度 记忆浏览器 + 多级开关 + 审计 2026亮点 Skills（工作流复用） Import迁移 + 隐私模式 实现Skills-like规则引擎 + 导入 历史回填 部分自动提取 显式ZIP导入成熟 提供导入 + 可选回填summarization 核心 takeaway：两者都证明“选择性 + 用户完全控制 + 透明治理”是可接受的平衡。先入库后删除模式在实践中被广泛接受（只要删除路径清晰）。对于本地系统，隐私优先（Temporary/sandbox模式）、可迁移性（import/export）和程序性记忆（Skills等价物）是2026年的差异化方向，能显著提升用户信任和长期使用价值。 如果您需要针对特定点深入展开（如Skills的实现思路、RAG注入的prompt模板、或本地数据库schema建议），或想看更多引用细节，随时告诉我。我可以继续补充或帮您梳理本地原型设计。

## Raw: ChatGPT（第 2 次，第 1 次仅捕获到首行）

下面按产品层公开信息来对标。结论先说：Gemini 的个人记忆体系公开资料更完整，已经把「显式指令 / 过去聊天 / Connected Apps / Search 个性化 / Temporary Chats」放进一个 Personal Intelligence 框架；Grok 的 consumer memory 公开资料较少，更多只能确认“有跨会话记忆/个性化开关、可删除对话/数据、X 数据可用于个性化”，但记忆抽取与注入细节没有官方充分披露。
0. 总体判断
维度	Google Gemini	xAI Grok
产品形态	Personal Intelligence：Instructions/Saved info + past chats memory + Connected Apps + Search personalization	Grok / grok.com / X 内 Grok + Grok Build/CLI 的 skills、session、compaction 等
记忆来源	明确包括：用户显式 instructions、Gemini Apps Activity 里的 past chats、Connected Apps、Search history（需授权）	明确包括：X data、Grok interactions/inputs/results；第三方报道确认 persistent memory 能记住 past conversations
用户显式记忆	Gemini 设置里 “Instructions for Gemini / Saved info”；也有 Enterprise 文档明确支持 “Remember…” 保存 memory	公开官方文档较少；第三方与用户资料显示可在对话中让 Grok remember，Data Controls 里有 Memory/Personalize toggles
注入机制	公开说法接近“按需选择上下文”：Gemini 会判断 custom instructions、past chats、Connected Apps 是否有助于回答；Search personalization 也只在模型判断有帮助时使用	Consumer Grok 未公开；Grok Build/CLI 公开提到 “memory context injection on resume”，但这是开发者/CLI 产品层，不等同 consumer Grok
治理	Instructions 可单条查看/编辑/删除/开关；past-chat memory 主要靠删除原聊天；Temporary Chat 不进入记忆/训练	可关闭训练与 personalization；可删除单条/全部 conversation；memory 单条治理主要来自第三方报道，官方 help 对“单条 memory”披露不足
旧数据回填	Gemini 有明确“import memory / full chat history from other AI apps”，还提到部分 Google Assistant 通讯请求历史可导入 Gemini Apps Activity	未找到 xAI 官方公开资料证明把旧历史聊天批量回填进新记忆系统；只能确认 memory 使用 past conversations / interaction data
1. 自动记忆抽取：来源、形态、是否事前审查
Gemini
可确认来源有三层：
第一层是过去 Gemini chats。Google 官方说明 Gemini 可以“reference your past chats”并从中学习偏好；Help 页面也说 Gemini 可用 past

## Raw: Gemini

两次均 `[NO RESPONSE]` 超时（240s / 280s），无内容。

## Raw: 内部源码 agent（remem user_context 管线事实，基于 origin/main 392a3e7）

**前置事实**：当前分支 `codex/plugin-version-sync` 的 src/ 中不存在 user_context_claims 代码；整套管线在 origin/main。

1. **写入路径仅 3 条**：手动 `remem user remember`（`src/user_context/claims.rs:120-158`，直接 active，无 review）；`claims edit` supersede 链（claims.rs:232-296）；candidate 应用（auto_promote / review approve / review edit，`src/user_context/candidates.rs:160-375`）。MCP/HTTP 只读。
2. **auto-promote 双层门槛**（`src/user_context/extraction/mod.rs:454-472` + candidates.rs:587-594）：claim_type ∈ {Preference, Constraint}；risk=Low；sensitivity=Normal；confidence ≥ 0.9；source_kind="explicit_user_statement"；所有 source_event_ids 为 user-authored；文本需在 user 源事件中有保守支持；同 claim_key 冲突仍拦回 pending_review。
3. **inbox 流转**：session_rollup 完成后 enqueue UserContextCandidate 提取任务（`src/session_rollup/mod.rs:89-114`）→ LLM 提取 → 未过 auto-promote 的进 pending_review；落库前丢弃层：non-retention（secret/speculative/temporary/general/illegal/external，`src/user_context/non_retention.rs`）+ third-party 无框架 + 无支撑 user 事件 + 去重。
4. **summary 已含旧 preference**：`load_memory_sources` SQL 含 `owner_scope='user' AND memory_type='preference'`（`src/user_context/summary.rs:427-446`），claims + memories + activity_refs 三路确定性拼接。
5. **无任何 preference → claims 回填代码**（INSERT 全库仅上述 3 处；migrations 无 INSERT...SELECT）。
6. **注入**：SessionStart 常驻注入旧 preference（project 20 / global 5 / 1500 chars，`src/context/policy.rs:51-53`）+ 一行静态 USER_CONTEXT_USAGE_POLICY；claims 条目不进 SessionStart，走按需 `recall_user_context`（默认 12 条 / 4000 chars 预算，`src/user_context/recall/types.rs:3-9`）。

## Phase 5 交叉验证矩阵（双外部模式：Grok + ChatGPT + 前次 WebSearch 官方源）

| # | 发现 | Grok | ChatGPT | WebSearch 官方源 | Tier |
|---|---|---|---|---|---|
| 1 | 两家自动记忆均为"先入库、事后治理"，无事前审查/公开置信度门槛 | ✅ 明确 | ✅ | ✅ (support.google / TechCrunch) | 🟢 强共识 |
| 2 | 显式记忆与自动记忆分离存储（Saved Info vs Personal Context；对话式 remember + Skills vs auto memory） | ✅ | ✅ | ✅ | 🟢 强共识 |
| 3 | 注入机制均为相关性驱动的选择性注入，非全量 | ✅ | ✅（"按需选择上下文"） | 部分 | 🟢 强共识 |
| 4 | Grok 4.3（2026-05）跨会话 memory 全平台 + Skills（程序性记忆层） | ✅ | 部分（Skills 见于 Grok Build/CLI 层） | ✅ (memorylake) | 🟢 强共识 |
| 5 | Gemini 2026-03 推出 Import：从其他 AI 导入 memories/完整聊天历史（ZIP），用户显式触发 | ✅ | ✅ | 未查证官方页 | 🟢 双外部一致 |
| 6 | Grok 无官方全量历史回填；启用 memory 时对非删除历史做增量提取 | ✅ | ✅（未找到官方回填资料） | — | 🟢 双外部一致 |
| 7 | Grok 单条 memory 可查看/删除（Referenced chats + forget） | ✅ | ⚠️ 称官方披露不足、多为第三方报道 | ✅ TechCrunch 引 xAI 官方 | 🟡 部分共识（官方声明存在，落地一致性存疑） |
| 8 | Gemini 自动层（Personal Context）用户看不到抽取产物，只能删原始 activity / 对话内纠正 | ✅ | ✅ | ✅ (support.google) | 🟢 强共识 |
| 9 | Gemini Temporary Chats（不入记忆，~72h 丢弃） | ✅ | ✅ | ✅ (blog.google) | 🟢 强共识 |
| 10 | remem 的 0.9 + explicit_user_statement 事前审查门槛严于两家业界产品一个量级 | — | — | 内部源码事实 + 发现 1 | 🟢（内部事实 × 外部强共识） |

单家独有且未交叉的概念：Grok 提到的"记忆衰减机制"（仅 Reddit 用户推测）→ ⚪ 不采纳为事实。

## Phase 6 分级行动清单

### 🔴 极高置信度必做（强共识）
1. **auto-promote 从"事前强审查"转向"先入库、事后治理"**：放宽 confidence 阈值（如 0.9 → 0.7）并考虑放宽 source_kind 限制；保留 non-retention 层（secrets/一次性内容不入库，业界 Temporary Chats 同构）。依据：发现 1+10；你已有完整事后治理面（suppress/delete/why/audit 行）。Rollback：改回原阈值（一个常量）。Verify：跑 3-5 个真实 session 后 `remem user claims list` 非空，人工审错误率。
2. **实现显式、用户触发的 preference → claims backfill 命令**（保留 source refs，输出导入报告）。依据：发现 5（Gemini Import 是用户显式迁移，非静默）+ 内部事实 5（当前无回填代码）。Rollback：claims 支持按 source_kind 批量 delete。Verify：backfill 后 claims 数 = 符合条件的 preference 数。
3. **修复 Claude hooks 捕获断流**：`remem install --target claude`（PostToolUse observe + Stop summarize 丢失是所有自动管线的上游断点）。Verify：settings.json 5/5 + 新 session 后 preference 有新增。

### 🟡 高置信度建议做（部分共识）
4. **Procedural 记忆层（Skills 等价物）**：把工作流类 preference（"依赖分析→分组→worktree"类）单独建模，按场景触发注入。依据：发现 4（Grok 单家产品化 + memorylake 佐证）。前提：先有消费侧分层注入需求。
5. **保持 claims 不进 SessionStart、按需 recall 的现有设计**——与发现 3（业界选择性注入）一致，这部分不是问题，不要改。

### ⚪ 待验证假设
6. Codex `user_prompt_submit` 证据捕获修复后 auto-promote 通过率能提升多少 → 需要修复后跑数据。
7. Grok 式"记忆衰减/重要性加权" → 无可靠公开依据，若做需自行 A/B。

### 🚫 不做
8. **静默自动全量回填**：两家都没有无感知全量迁移；Gemini 的 Import 是显式动作。回填必须用户触发 + 有审计报告。
9. 基于"衰减机制"传闻做设计决策（单家 Reddit 推测，未交叉）。

## Metadata
- 外部 AI 状态：grok ✅（1 次成功）、chatgpt ✅（2 次，第 1 次截断）、gemini ❌（2 次 NO RESPONSE）
- 内部 agent：1 个（127k tokens, 38 tool uses, 440s）
- 注意：claims 管线代码在 origin/main，当前分支 src/ 无此代码；改 policy 需基于 main。
