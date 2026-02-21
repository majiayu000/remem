# remem

Claude Code 的持久化记忆系统。单个 Rust 二进制，零配置，一键安装。

跨会话记住你的架构决策、代码模式、用户偏好和项目上下文——让每次新会话都能从上次离开的地方继续。

## 工作原理

remem 通过 Claude Code 的 Hooks 系统，在你正常工作时静默运行：

```
你正常使用 Claude Code
        │
        ├─ SessionStart      → 注入历史记忆到上下文
        ├─ UserPromptSubmit  → 注册会话 + 自动 flush 残留队列
        ├─ PostToolUse       → 记录工具操作（入队，<1ms，不阻塞）
        └─ Stop              → 6ms 返回，后台总结 + 压缩
```

**你不需要做任何事情**——记忆的采集、提炼、检索全自动。

## 安装

```bash
# 编译
cargo build --release

# 一键安装（自动配置 hooks + MCP 到 ~/.claude/settings.json）
./target/release/remem install
```

安装完成后重启 Claude Code，remem 开始工作。

### 卸载

```bash
remem uninstall    # 移除 hooks 和 MCP 配置，数据保留
```

## 架构

```
┌───────────────────────────────────────────────────────────┐
│                    Claude Code Hooks                       │
│                                                            │
│  SessionStart ──────→ context       (注入历史记忆)          │
│  UserPromptSubmit ──→ session-init  (注册会话+flush残留)    │
│  PostToolUse ───────→ observe       (过滤+入队 SQLite)      │
│  Stop ──────────────→ summarize     (3层gate+后台worker)    │
└──────────────┬──────────────────────────┬──────────────────┘
               │                          │
               ▼                          ▼
┌──────────────────────┐  ┌──────────────────────────────────┐
│  MCP Server (stdio)  │  │  Background Worker (detached)     │
│                      │  │                                    │
│  search ─→ 全文搜索   │  │  1. flush (批量→观察, ≤15/批)     │
│  get_observations    │  │  2. compress (>100条→自动合并)     │
│  timeline            │  │  3. summarize (会话总结,增量合并)   │
│  save_memory         │  │                                    │
│                      │  │  超时保护: 180s 全局上限            │
└──────────┬───────────┘  └─────────────┬────────────────────┘
           │                            │
           ▼                            ▼
┌───────────────────────────────────────────────────────────┐
│              ~/.remem/remem.db (SQLite + WAL)              │
│                                                            │
│  pending_observations → observations → compressed          │
│  (工具事件队列)          (AI 提炼记忆)   (旧记忆合并)        │
│                                                            │
│  sdk_sessions         session_summaries    FTS5 全文索引    │
│  summarize_cooldown   (项目级速率限制)                       │
└───────────────────────────────────────────────────────────┘
```

### 模块一览（~3000 行 Rust）

| 模块 | 行数 | 职责 |
|------|------|------|
| `context.rs` | 497 | 上下文渲染：时间线 + 渐进式披露 + token 经济统计 |
| `db.rs` | 582 | 数据模型 + 写操作 + 速率限制 + 数据清理 |
| `db_query.rs` | 367 | 读查询：FTS 搜索、时间线、按 ID 获取 |
| `summarize.rs` | 433 | 3 层 gate + 后台 worker + 会话总结 + 长期压缩 |
| `observe.rs` | 391 | Bash 过滤 + 事件入队 + 批量 flush + 增量去重 |
| `mcp.rs` | 225 | MCP Server：search / timeline / get_observations / save_memory |
| `install.rs` | 210 | 自动配置 hooks + MCP 到 settings.json |
| `ai.rs` | 142 | AI 调用：HTTP-first + CLI fallback + model 名映射 |
| `main.rs` | 116 | CLI 入口：11 个子命令 |
| `log.rs` | 70 | 日志：文件 + stderr，Timer 计时 |
| `search.rs` | 33 | 搜索入口：FTS 全文 / 按类型过滤 |

## 数据流

### 1. 观察采集（PostToolUse → observe）

```
工具调用 ──→ 类型检查 ──→ Bash 过滤 ──→ 入队 SQLite
              │              │
              │              └─ 跳过: git status/log/diff, ls, cat,
              │                      npm install, cargo build 等只读命令
              │
              └─ 只接收: Write, Edit, NotebookEdit, Bash
                 跳过: Read, Glob, Grep, Task 等读取工具
```

每个入队事件存储：session_id, project, tool_name, tool_input, tool_response（截断至 4KB）。

### 2. 批量提炼（Stop → summarize → flush）

```
Stop hook 触发
       │
       ├─ Gate 1: pending < 3 → 跳过（过滤短命 session）
       ├─ Gate 2: 项目冷却期 300s → 跳过（防重复）
       ├─ Gate 3: message hash 相同 → 跳过（防重复内容）
       │
       ▼ 通过所有 gate
  spawn 后台 worker（6ms 返回）
       │
       ├─ Worker 再次检查 gate 2+3（防并行竞争）
       ├─ 提前记录冷却期（占位）
       │
       ▼
  flush_pending（≤15 事件/批）
       │
       ├─ 注入已有记忆（delta 去重）
       ├─ 单次 AI 调用 → 结构化观察
       ├─ 文件重叠检测 → 标记旧观察为 stale
       │
       ▼
  summarize（会话总结）
       │
       ├─ 注入同 session 旧 summary（增量合并）
       ├─ AI 生成 → 替换旧 summary
       │
       ▼
  maybe_compress（长期压缩）
       │
       └─ >100 活跃观察 → 最旧 30 条合并为 1-2 条精简摘要
```

### 3. 上下文注入（SessionStart → context）

```
新会话启动
       │
       ▼
  加载最近 50 条观察 + 10 条 session 摘要
       │
       ├─ 高价值优先: decision > bugfix > feature > 其他
       ├─ 前 10 条完整展示（narrative），其余表格行
       ├─ stale 观察限制为活跃数的 20%
       │
       ▼
  渲染到 stdout → Claude Code 注入 CLAUDE.md
       │
       ├─ 按日期+session 分组的时间线
       ├─ Token 经济统计（读取成本 vs 生成投入）
       └─ 最近 3 条 session summary（request/completed/decisions/learned）
```

### 4. 残留队列回收（UserPromptSubmit → session_init）

```
新消息提交
       │
       ├─ 注册/更新 session
       │
       ▼
  扫描同项目超 10 分钟的残留 pending
       │
       └─ 自动 flush → 防止低活跃 session 的观察丢失
```

## 记忆生命周期

```
工具操作 ──→ pending_observations (原始队列, ≤4KB/条)
                    │
                    ▼ flush（≤15条/批, 单次 AI 调用）
             observations (结构化记忆)
                    │
        ┌───────────┼───────────┐
        ▼           ▼           ▼
     active      stale      compressed
   (正常展示)  (文件覆盖,   (>100条时
              降权展示)    自动合并)
                              │
                              ▼ 90 天后
                           deleted (cleanup 命令清理)
```

- **增量 delta**: flush 时注入最近 10 条已有记忆，AI 自动跳过重复内容
- **文件重叠失效**: 新操作覆盖旧文件时，旧观察自动标记为 stale
- **时间衰减**: FTS 搜索按 rank × 时间衰减排序，stale 额外降权
- **自动压缩**: 项目超过 100 条观察时，保留最新 50 条，最旧 30 条合并为 1-2 条精简摘要
- **TTL 清理**: compressed 状态超 90 天的记录由 `cleanup` 命令删除

## 速率限制（防 token 浪费）

短命进程模型（每次 hook 独立进程）无法通过内存状态去重，remem 用 SQLite 实现 3 层 gate：

| 层 | 机制 | 拦截场景 |
|----|------|----------|
| Gate 1 | `pending < 3` 跳过 | 短命 session（只做了 1-2 个操作就结束） |
| Gate 2 | 项目冷却期 300s | 同项目快速连续 summarize |
| Gate 3 | message hash 去重 | 相同 assistant message 不重复处理 |
| Worker 双检 | 进入 worker 后再验 Gate 2+3 | 多 worker 并行竞争 |
| 提前占位 | AI 调用前记录冷却期 | 防止并行 worker 同时通过 gate |

`summarize_cooldown` 表存储每个 project 的最近一次 summarize 时间和 message hash。

## 数据清理

```bash
remem cleanup    # 一键清理所有垃圾数据
```

清理内容：
- 孤立 summary（`mem-*` 前缀但无对应 observation）
- 重复 summary（同 session+project 只保留最新一条）
- 过期 pending（超 1 小时未处理）
- 过期 compressed（超 90 天的压缩记录）

## AI 调用

```
              ┌─ ANTHROPIC_API_KEY 存在？
              │
         是 ──┤──→ HTTP API 直连（2-5s）
              │         │ 失败
              │         ▼
         否 ──┴──→ claude -p CLI（30-60s）
```

- **model 映射**: `REMEM_MODEL=haiku` → `claude-haiku-4-5-20251001`（HTTP 用完整 ID，CLI 用短名）
- **超时**: 单次 AI 调用 90s，整个 worker 180s
- **4 个 prompt**: observation（采集）、summary（会话总结）、compress（长期压缩）

## MCP Server

通过 stdio 传输的 MCP server，提供 4 个工具：

| 工具 | 说明 |
|------|------|
| `search` | 全文搜索（FTS5）+ 项目/类型过滤，返回 ID+标题 |
| `get_observations` | 按 ID 获取完整记忆（narrative, facts, concepts, files） |
| `timeline` | 时间线查询：指定锚点前后的观察序列 |
| `save_memory` | 手动保存重要记忆（架构决策、用户偏好等） |

推荐工作流：`search(query)` → 找到相关 ID → `get_observations(ids)` 获取完整内容。

## 项目识别

项目名从工作目录提取最后两级路径，防止同名目录碰撞：

```
/Users/foo/code/my-app       → code/my-app
/Users/foo/personal/my-app   → personal/my-app   （不同项目）
/Users/foo/Desktop/code/AI/tools/remem → tools/remem
```

## 环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `REMEM_DATA_DIR` | `~/.remem` | 数据目录（DB + 日志） |
| `REMEM_MODEL` | `haiku` | AI 模型（haiku/sonnet/opus 或完整 model ID） |
| `REMEM_EXECUTOR` | `auto` | AI 执行器：`auto`（HTTP 优先）/ `http` / `cli` |
| `ANTHROPIC_API_KEY` | - | HTTP 模式需要（也支持 `ANTHROPIC_AUTH_TOKEN`） |
| `ANTHROPIC_BASE_URL` | `https://api.anthropic.com` | 自定义 API 端点 |
| `REMEM_DEBUG` | - | 设置后输出 debug 日志 |
| `REMEM_CONTEXT_OBSERVATIONS` | `50` | 上下文加载的观察数上限 |
| `REMEM_CONTEXT_FULL_COUNT` | `10` | 完整展示（含 narrative）的观察数 |
| `REMEM_CONTEXT_SESSION_COUNT` | `10` | 展示的会话摘要数 |
| `REMEM_CONTEXT_OBSERVATION_TYPES` | `bugfix,feature,...` | 加载的观察类型 |
| `REMEM_CONTEXT_FULL_FIELD` | `narrative` | 完整展示使用的字段（narrative/facts） |
| `REMEM_CONTEXT_SHOW_READ_TOKENS` | `true` | 显示读取 token 统计 |
| `REMEM_CONTEXT_SHOW_WORK_TOKENS` | `true` | 显示工作 token 统计 |
| `REMEM_CONTEXT_SHOW_LAST_SUMMARY` | `true` | 显示最近 session summary |
| `REMEM_CLAUDE_PATH` | `claude` | Claude CLI 路径 |

## 命令

```bash
remem install                          # 安装 hooks + MCP
remem uninstall                        # 卸载配置
remem context --cwd . [--color]        # 手动生成上下文（调试用）
remem mcp                              # 启动 MCP server
remem flush --session-id <id> --project <name>  # 手动 flush
remem cleanup                          # 清理垃圾数据
```

## 数据库 Schema

```sql
-- 工具事件队列
pending_observations (session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch)

-- 结构化记忆
observations (memory_session_id, project, type, title, subtitle, narrative, facts, concepts,
              files_read, files_modified, status[active|stale|compressed], discovery_tokens)

-- 会话摘要
session_summaries (memory_session_id, project, request, completed, decisions, learned,
                   next_steps, preferences, discovery_tokens)

-- 会话映射
sdk_sessions (content_session_id → memory_session_id, project, prompt_counter)

-- 速率限制
summarize_cooldown (project, last_summarize_epoch, last_message_hash)

-- 全文索引
observations_fts (title, subtitle, narrative, facts, concepts)  -- FTS5, 自动同步
```

## 设计决策

- **短命进程模型**: 每次 hook 调用独立进程，零共享状态，<6ms 响应，不阻塞 Claude Code
- **SQLite 约束补偿**: 无内存 Map 去重能力，用 DB 表（summarize_cooldown）模拟速率限制
- **HTTP-first AI 调用**: HTTP API 直连 2-5s vs `claude -p` CLI 30+s，性能差 6-12x
- **Stop hook 异步化**: dispatcher 6ms 返回，`std::process::Command` spawn 独立 worker
- **SQLite 单文件 + WAL**: 零依赖，FTS5 全文搜索，WAL 支持并发读写
- **队列批处理**: PostToolUse 只入队（<1ms），Stop 时一次 AI 调用处理 ≤15 事件
- **决策优先**: summary 字段按 decisions > completed > learned 排序，架构知识最有价值
- **Schema 版本控制**: `PRAGMA user_version` 跳过重复 migration，减少每次 hook 的 DB 开销
- **两级项目名**: `parent/dirname` 防止 `/work/api` 和 `/personal/api` 碰撞

## License

MIT
