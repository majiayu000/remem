# Context Budget Design Note (2026-04-29)

## 背景

用户在 Codex `SessionStart` context 中看到：

```text
50 memories loaded.
```

这不是数据库里的总记忆数，而是 `remem context` 在 SessionStart 时从候选记忆中筛选后注入的上限。当前实现把偏好、决策、发现等记忆放进同一个 `memories` 池，再统一截断为 50 条。

一次本机复现：

```bash
cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/computer
```

关键结果：

```text
[context] DONE 1120ms project=/Users/lifcc/Desktop/code/AI/tools/computer memories=50 summaries=2 workstreams=0
```

最终 index 组成：

```text
Preferences (46)
Decisions (2)
Discoveries (2)
```

这说明 `50` 作为性能上限暂时可接受，但作为单一内容预算不合理：偏好已经有独立 `Your Preferences` section，却又占满主 memory index，导致 decision / bugfix / discovery 等任务记忆被挤出。

## 当前代码边界

- `src/context/query.rs`
  - `CONTEXT_MEMORY_LIMIT: usize = 50`
  - `RECENT_MEMORY_FETCH_LIMIT = 100`
  - `BASENAME_SEARCH_LIMIT = 20`
  - `MAX_SELF_DIAGNOSTIC_MEMORIES = 2`
  - `load_project_memories()` 从 recent + basename search 合并、去重、限制自诊断记忆，然后 `take(50)`。
- `src/preference/render.rs`
  - 项目偏好上限 20。
  - 全局偏好上限 10。
  - 输出到独立 `## Your Preferences (always apply these)` section。
  - 输出字符预算 `MAX_CHARS = 1500`。
- `src/context/render.rs`
  - 先渲染 preferences，再渲染 core/index/sessions。
  - 末尾显示 `loaded.memories.len()`。
- `docs/ARCHITECTURE.md`
  - 仍记录 `REMEM_CONTEXT_OBSERVATIONS` 等 env vars。
  - 当前代码没有读取这些 context env vars，属于文档和实现漂移。

## multi-ai-research 初步结论

2026-04-29 使用 `multi-ai-research` 外部交叉调研：

- Gemini: 成功。
- Grok: 成功。
- DeepSeek: 成功。
- ChatGPT web adapter: 失败，原因是没有找到 send button。
- Claude CLI: 失败，原因是未登录。

三路有效结果一致：

1. 不建议保留单一硬编码 `CONTEXT_MEMORY_LIMIT = 50` 作为所有记忆类型的共同预算。
2. 偏好应该由独立 preference section 管理，不应继续占用 main memory index/core 的主要名额。
3. 应该引入 typed/section budgets，至少拆分 main memory、core、session、self-diagnostic、preference project/global/char limit。
4. 应该提供 env override，并让 `docs/ARCHITECTURE.md` 与代码保持一致。
5. 需要用测试覆盖 preference flood、env override、dedupe/no-starvation。

## 建议设计

第一阶段不做复杂重排，先修正当前明显问题：

```rust
struct ContextLimits {
    candidate_fetch_limit: usize,       // default 120
    memory_index_limit: usize,          // default 50, non-preference memories only
    core_item_limit: usize,             // default 6
    core_char_limit: usize,             // default 3000
    session_limit: usize,               // default 5
    self_diagnostic_limit: usize,       // default 2
    preference_project_limit: usize,    // default 20
    preference_global_limit: usize,     // default 10
    preference_char_limit: usize,       // default 1500
}
```

默认策略：

- `preference` 从 main memory index/core 的候选池中排除。
- `Your Preferences` section 继续独立渲染，但预算由 `ContextLimits` 控制。
- 候选召回可以比最终注入更宽，默认 120 条；最终 main index 仍只注入 50 条非 preference 记忆。
- main memory index 默认保留 50 条非 preference 记忆。
- core 默认保留 6 条高分记忆，并受字符预算限制。
- sessions 默认保留 5 条最近 session summary。
- self-diagnostic 默认最多 2 条。
- 类型软预算作为第二阶段：decision / bugfix / architecture / discovery 至少有展示机会，剩余额度再按当前分数回填。

环境变量建议：

| New variable | Default | Notes |
| --- | ---: | --- |
| `REMEM_CONTEXT_CANDIDATE_FETCH_LIMIT` | `120` | 候选召回上限，最终仍会按 section budget 裁剪 |
| `REMEM_CONTEXT_MEMORY_INDEX_LIMIT` | `50` | main index 非 preference 记忆上限 |
| `REMEM_CONTEXT_CORE_ITEM_LIMIT` | `6` | core section 条数上限 |
| `REMEM_CONTEXT_CORE_CHAR_LIMIT` | `3000` | core section 字符预算 |
| `REMEM_CONTEXT_SESSION_COUNT` | `5` | session summary 上限 |
| `REMEM_CONTEXT_SELF_DIAGNOSTIC_LIMIT` | `2` | self-diagnostic 记忆上限 |
| `REMEM_CONTEXT_PREFERENCE_PROJECT_LIMIT` | `20` | 项目偏好查询上限 |
| `REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT` | `10` | 全局偏好查询上限 |
| `REMEM_CONTEXT_PREFERENCE_CHAR_LIMIT` | `1500` | preference section 字符预算 |

兼容策略：

- `REMEM_CONTEXT_OBSERVATIONS` 保留为 deprecated alias，映射到 `REMEM_CONTEXT_MEMORY_INDEX_LIMIT`。
- `docs/ARCHITECTURE.md` 删除或标注已经不存在的 `REMEM_CONTEXT_FULL_COUNT`、`REMEM_CONTEXT_OBSERVATION_TYPES`、`REMEM_CONTEXT_FULL_FIELD`、token display vars，除非先补实现。

## 第一阶段验收

建议最小测试集：

- `preference_flood_does_not_starve_core_memories`
  - 构造大量 preference 和少量 decision/discovery。
  - 断言 preference 只出现在 `Your Preferences`，main index/core 仍保留 decision/discovery。
- `context_limits_env_override_is_respected`
  - 设置 env override。
  - 断言 memory/session/preference budget 都按 override 生效。
- `preferences_rendered_separately_not_in_index`
  - 断言同一 preference 不重复进入 main memory index。
- `deprecated_context_observations_alias_still_works`
  - 断言旧 env alias 仍能控制 main index limit。

建议 smoke：

```bash
cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/computer
```

验收重点不是只看 `memories=N`，而是看类型构成：

- `Your Preferences` 仍存在。
- `## Core` 不被 preference flood 占满。
- `## Index` 中 decision / bugfix / discovery 不被 preference 挤出。

## 开源项目对照

2026-04-29 开 3 个子 agent 做只读调研：

- Agent A: 通用 agent memory 框架，覆盖 Mem0、Zep/Graphiti、Letta/MemGPT、LangMem/LangGraph memory。
- Agent B: 编程助手 / SessionStart / MCP memory，覆盖 Claude Code auto memory、Claude-Mem、Engram、Basic Memory、Supermemory MCP、OpenAI Agents SDK Memory。
- Agent C: RAG / 本地知识库 / 长期记忆检索，覆盖 Chroma、LlamaIndex、LangChain/TimeWeighted/MMR、Letta archival/core split。

三条线的共同结论：

1. 主流系统不把启动上下文设计成一个所有类型共享的 flat item limit。
2. 小量高价值内容常驻：profile、preferences、core memory、summary、index。
3. 长尾记忆按需检索：search / timeline / get details / topic files / rollout summaries。
4. 预算按 section、类型、字符/token、scope/filter 拆开。
5. 启动注入偏向 compact index，不默认注入全文。
6. 检索链路通常是先宽召回，再 rerank/filter/compress，最后按最终 prompt budget 裁剪。

## 代表项目做法

| 项目 | 关键做法 | 对 remem 的启发 |
| --- | --- | --- |
| Mem0 | `search` 使用 scoped filters、`top_k`、threshold、rerank；由应用把结果注入 prompt，不是全量常驻。 | main context 应该是 scoped retrieval/index，不是所有 memory 的扁平 dump。 |
| Zep / Graphiti | context template 支持 `%{edges limit=10}`、`%{entities limit=5}`、`%{user_summary}`；事实、实体、summary 分 section。 | 直接借鉴 section limit，把 preference/profile 和 facts/decisions 拆池。 |
| Letta / MemGPT | core memory blocks 常驻；archival memory 按需 search；memory blocks/files/archival/external RAG 有不同 size/count 建议。 | `Your Preferences` 类似 core/profile；decision/discovery 更接近 archival/index，不能互相抢预算。 |
| LangMem / LangGraph | long-term memory 用 namespace/store；agent 可用 manage/search tools；后台 manager 可异步抽取和合并。 | 保留 SessionStart 小索引，同时鼓励工具链按需 `search -> get_observations`。 |
| Claude Code auto memory | 每个项目有 `MEMORY.md` entrypoint 和 topic files；启动只加载 `MEMORY.md` 前 200 行或 25KB，topic files 不启动全量加载。 | remem 的 SessionStart 输出应该是入口地图，不是百科全书。 |
| Claude-Mem | MCP search tools 是三层 workflow：`search` 紧凑索引、`timeline` 上下文、`get_observations` 详情。 | remem 已有相同工具，应把 SessionStart 更明确地定位为索引入口。 |
| Basic Memory | Markdown + SQLite knowledge graph；通过 `search_notes`、`build_context(memory://...)`、`read_note` 按需展开。 | 可以借鉴 knowledge graph / path-oriented context，但不需要启动时塞入全部。 |
| OpenAI Agents SDK Memory | 启动注入小 `memory_summary.md`，再搜索 `MEMORY.md`，需要时才打开 rollout summaries。 | 与 remem 的 `Core + Index + Sessions` 结构一致，但应保持 compact 和可追溯。 |
| Chroma / LlamaIndex / LangChain | 候选召回后做 metadata filter、similarity cutoff、rerank、MMR/diversity、time-weighted rerank、contextual compression。 | 第二阶段可以增加轻量 MMR/topic diversity，避免同一主题重复占满 50 条。 |

## Primary Sources

- Mem0 search: https://docs.mem0.ai/core-concepts/memory-operations/search
- Zep context templates: https://help.getzep.com/context-templates
- Letta context hierarchy: https://docs.letta.com/guides/core-concepts/memory/context-hierarchy
- LangMem README: https://github.com/langchain-ai/langmem
- Claude Code memory: https://code.claude.com/docs/en/memory
- Claude-Mem README: https://github.com/thedotmack/claude-mem
- Basic Memory README: https://github.com/basicmachines-co/basic-memory
- OpenAI Agents SDK memory: https://openai.github.io/openai-agents-python/sandbox/memory/
- Chroma agentic memory: https://docs.trychroma.com/guides/build/agentic-memory
- LlamaIndex node postprocessors: https://developers.llamaindex.ai/python/framework/module_guides/querying/node_postprocessors/

## 最终建议

`CONTEXT_MEMORY_LIMIT = 50` 可以保留为兼容语义，但应降级为“main non-preference index limit”的默认值。真正的模型应该是：

```text
SessionStart context
  ├─ Preferences/Profile: 独立预算，始终 apply
  ├─ Core: 6 条左右最高价值非 preference 记忆，受字符预算限制
  ├─ Memory Index: 50 条以内非 preference 紧凑索引
  ├─ Workstreams: 当前 active workstream
  └─ Sessions: 5 条左右 recent session summary

Details
  └─ search -> timeline -> get_observations 按需展开
```

第一阶段实施顺序：

1. 增加 `ContextLimits` 和 env parsing。
2. `load_project_memories()` 默认排除 `preference`。
3. `render_preferences()` 改为读取 `ContextLimits`。
4. `render_core()` / `query_recent_summaries()` 改为读取 `core_item_limit`、`core_char_limit`、`session_limit`。
5. 同步 `docs/ARCHITECTURE.md`，清理未实现 env。
6. 加 preference flood / env override / deprecated alias / UTF-8 char budget 测试。

第二阶段再评估：

1. 类型软预算：decision / bugfix / architecture / discovery。
2. 轻量 MMR/topic diversity：同 topic/cluster 默认最多 1-2 条。
3. Token cost 或字符 cost 显示，避免只看 item count。
