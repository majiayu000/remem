# remem 安装配置待办

## 状态

- [x] 编译二进制 (`cargo build --release`)，3.8MB
- [x] SDK feature 支持 (`cargo build --release --features sdk`)
- [x] 数据库 `~/.claude-mem/claude-mem.db` 已存在
- [ ] Claude Code Hooks 配置
- [ ] MCP Server 注册
- [ ] 环境变量配置
- [ ] 集成测试 (`test.sh`)

## 1. 配置 Hooks

文件：`~/.claude/hooks.json`

```json
{
  "hooks": {
    "SessionStart": [
      {
        "type": "command",
        "command": "/Users/lifcc/Desktop/code/AI/tools/remem/target/release/remem context --cwd $CWD --session-id $SESSION_ID"
      }
    ],
    "UserPromptSubmit": [
      {
        "type": "command",
        "command": "/Users/lifcc/Desktop/code/AI/tools/remem/target/release/remem session-init"
      }
    ],
    "PostToolUse": [
      {
        "type": "command",
        "command": "/Users/lifcc/Desktop/code/AI/tools/remem/target/release/remem observe"
      }
    ],
    "Stop": [
      {
        "type": "command",
        "command": "/Users/lifcc/Desktop/code/AI/tools/remem/target/release/remem summarize"
      }
    ]
  }
}
```

## 2. 注册 MCP Server

文件：`~/.claude/settings.json` 中添加：

```json
{
  "mcpServers": {
    "remem": {
      "command": "/Users/lifcc/Desktop/code/AI/tools/remem/target/release/remem",
      "args": ["mcp"]
    }
  }
}
```

MCP 提供 4 个工具：search, timeline, get_observations, save_memory

## 3. 环境变量

### 必须（HTTP 模式）

| 变量 | 说明 |
|------|------|
| `ANTHROPIC_API_KEY` 或 `ANTHROPIC_AUTH_TOKEN` | Anthropic API 密钥 |

### 可选

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `CLAUDE_MEM_MODEL` | `claude-sonnet-4-5-20250929` | observe/summarize 用的模型 |
| `ANTHROPIC_BASE_URL` | `https://api.anthropic.com` | API 基础 URL |
| `CM_EXECUTOR_MODE` | `http` | `http` / `sdk` / `composite` |
| `CM_SDK_TIMEOUT_SECS` | `120` | SDK 模式超时（5-600秒） |
| `CM_CLAUDE_CODE_PATH` | PATH 中的 claude | SDK 模式 CLI 路径 |
| `CM_SDK_OBSERVER_CWD` | 无 | SDK 模式工作目录 |

### Context 显示

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `CLAUDE_MEM_CONTEXT_OBSERVATIONS` | `50` | 加载的 observation 数量 |
| `CLAUDE_MEM_CONTEXT_FULL_COUNT` | `5` | 展示完整内容的条数 |
| `CLAUDE_MEM_CONTEXT_SESSION_COUNT` | `10` | 显示的 session 数量 |
| `CLAUDE_MEM_CONTEXT_OBSERVATION_TYPES` | `bugfix,feature,refactor,discovery,decision,change` | 过滤类型 |
| `CLAUDE_MEM_CONTEXT_SHOW_READ_TOKENS` | `true` | 显示读取 token |
| `CLAUDE_MEM_CONTEXT_SHOW_WORK_TOKENS` | `true` | 显示工作 token |
| `CLAUDE_MEM_CONTEXT_SHOW_LAST_SUMMARY` | `true` | 显示最近 summary |
| `CLAUDE_MEM_CONTEXT_FULL_FIELD` | `narrative` | 完整展示的字段 |

## 4. 架构概览

```
Session 开始 → context 生成历史摘要注入对话
用户提问 → session-init 记录 session
工具调用后 → observe 调 AI 提取知识点存 DB
Session 结束 → summarize 调 AI 生成总结存 DB
对话中随时 → MCP search/get_observations 按需查询记忆
```

5 个子命令：

| 子命令 | Hook | 作用 |
|--------|------|------|
| `context` | SessionStart | 生成历史上下文 markdown |
| `session-init` | UserPromptSubmit | 初始化 session 记录 |
| `observe` | PostToolUse | AI 分析工具调用提取知识 |
| `summarize` | Stop | AI 生成 session 总结 |
| `mcp` | 常驻 | MCP server（4 个工具） |

## 5. 执行器模式

| 模式 | 环境变量值 | 说明 |
|------|-----------|------|
| HTTP | `http`（默认） | 直连 Anthropic API，需要 API key |
| SDK | `sdk` | 通过 Claude Code CLI 子进程，用订阅计费 |
| Composite | `composite` | SDK 优先，失败 fallback 到 HTTP |

SDK/Composite 需要 `--features sdk` 编译。

## 6. 与 claude-mem (TS) 的区别

| 特性 | claude-mem (TS) | remem (Rust) |
|------|----------------|-------------|
| 运行时 | Node.js + Bun | 单二进制 |
| 安装 | npm + plugin marketplace | cargo build |
| Worker | HTTP API (37777) | 无（内嵌） |
| 子进程 | 多个 | 零 |
| 搜索 | SQLite FTS5 + Chroma 向量 | SQLite FTS5 |
| MCP | Node MCP SDK | rmcp crate |
| 二进制大小 | ~50MB+ (node_modules) | 3.8MB |
| 版本 | v10.1.0 (生产) | v0.1.0 (早期) |
