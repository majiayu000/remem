# remem

Claude Code 的持久化记忆系统。单个 Rust 二进制，零配置，一键安装。

跨会话记住你的架构决策、代码模式、用户偏好和项目上下文——让每次新会话都能从上次离开的地方继续。

## 工作原理

remem 通过 Claude Code 的 Hooks 系统，在你正常工作时静默运行：

```
你正常使用 Claude Code
        │
        ├─ SessionStart  → 注入历史记忆到上下文
        ├─ UserPromptSubmit → 注册会话
        ├─ PostToolUse   → 记录工具操作（入队，不阻塞）
        └─ Stop          → 6ms 返回，后台总结 + 压缩
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
┌─────────────────────────────────────────────────┐
│               Claude Code Hooks                  │
│                                                  │
│  SessionStart ──→ context     (注入历史记忆)      │
│  UserPromptSubmit → session-init (注册会话)       │
│  PostToolUse ────→ observe    (队列入 SQLite)     │
│  Stop ───────────→ summarize  (6ms 返回)         │
└─────────────┬──────────────────────┬─────────────┘
              │                      │
              ▼                      ▼
┌────────────────────┐  ┌────────────────────────┐
│  MCP Server        │  │  Background Worker      │
│  实时搜索/查询记忆  │  │  1. flush (批量→观察)    │
│                    │  │  2. compress (自动压缩)  │
│                    │  │  3. summarize (会话总结) │
└────────┬───────────┘  └──────────┬─────────────┘
         │                         │
         ▼                         ▼
┌─────────────────────────────────────────────────┐
│            ~/.remem/remem.db (SQLite)            │
│                                                  │
│  pending → observations → compressed             │
│  (原始队列)  (AI 提炼记忆)  (旧记忆合并)           │
│                                                  │
│  sessions    summaries    FTS5 全文索引           │
└─────────────────────────────────────────────────┘
```

### 模块一览（~3000 行 Rust）

| 模块 | 职责 |
|------|------|
| `main.rs` | CLI 入口，10 个子命令 |
| `observe.rs` | 工具事件队列 + 批量 AI 处理 + 增量去重 |
| `summarize.rs` | 异步 Stop hook + 会话总结 + 长期压缩 |
| `context.rs` | 上下文渲染（时间线 + 渐进式披露） |
| `db.rs` | SQLite 数据层 |
| `ai.rs` | AI 调用（HTTP-first, CLI fallback, 90s 超时） |
| `mcp.rs` | MCP Server（search/get） |
| `install.rs` | 自动配置 hooks + MCP |

## 记忆生命周期

```
工具操作 ──→ pending_observations (原始队列)
                    │
                    ▼ flush（批量 AI 调用）
             observations (结构化记忆)
                    │
        ┌───────────┼───────────┐
        ▼           ▼           ▼
     active      stale      compressed
   (正常展示)  (文件覆盖,   (>100条时
              降权展示)    自动合并)
```

- **增量 delta**: flush 时注入已有记忆，AI 自动跳过重复内容
- **文件重叠失效**: 新操作覆盖旧文件时，旧观察自动标记为 stale
- **时间衰减**: 按时间倒序排列，高价值类型（decision, bugfix）优先展示
- **自动压缩**: 项目超过 100 条观察时，最旧的 30 条合并为 1-2 条精简摘要

## 环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `REMEM_DATA_DIR` | `~/.remem` | 数据目录（DB + 日志） |
| `REMEM_MODEL` | `haiku` | AI 模型 |
| `REMEM_EXECUTOR` | `auto` | AI 执行器（`auto`/`cli`/`http`） |
| `ANTHROPIC_API_KEY` | - | HTTP 模式需要 |
| `REMEM_DEBUG` | - | 设置后输出 debug 日志 |
| `REMEM_CONTEXT_OBSERVATIONS` | `50` | 上下文加载的观察数上限 |
| `REMEM_CONTEXT_FULL_COUNT` | `10` | 完整展示的观察数 |
| `REMEM_CONTEXT_SESSION_COUNT` | `10` | 展示的会话摘要数 |

## 命令

```bash
remem install           # 安装 hooks + MCP
remem uninstall         # 卸载配置
remem context --cwd .   # 手动生成上下文（调试用）
remem mcp               # 启动 MCP server
remem flush --session-id <id> --project <name>  # 手动 flush
```

## 设计决策

- **HTTP-first AI 调用**: HTTP API 直连 2-5s vs `claude -p` CLI 30+s，性能差 6-12 倍
- **Stop hook 异步化**: dispatcher 6ms 返回，worker 后台执行，用户零感知
- **SQLite 单文件**: 零依赖，无需数据库服务，FTS5 全文搜索
- **队列批处理**: PostToolUse 只入队（<1ms），Stop 时一次 AI 调用处理所有事件
- **决策优先 schema**: summary 字段按 decisions > completed > learned 排序，架构知识最有价值

## License

MIT
