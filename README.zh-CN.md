# remem：Claude Code 和 Codex CLI 的本地优先记忆层

> 别再在每个新的 coding-agent 会话里重新解释你的项目。

语言： [English](README.md) | **简体中文**

`remem` 是一个 Rust 单二进制程序，会在 Claude Code 和 Codex CLI
会话间自动捕获、提炼、搜索并注入项目记忆。它把决策、Bug 修复原因、
项目模式和偏好通过 hooks、MCP、CLI 和 REST 持续带回会话，不需要外部数据库。

[![CI](https://github.com/majiayu000/remem/actions/workflows/ci.yml/badge.svg)](https://github.com/majiayu000/remem/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/majiayu000/remem?sort=semver)](https://github.com/majiayu000/remem/releases/latest)
[![crates.io](https://img.shields.io/crates/v/remem-ai)](https://crates.io/crates/remem-ai)
[![npm](https://img.shields.io/npm/v/%40remem-ai%2Fremem)](https://www.npmjs.com/package/@remem-ai/remem)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

![Remem 记忆接续演示 — 新会话直接接上上周的 bug 修复](assets/remem-recall-demo.gif)

*真实 Claude Code 会话（demo 仓库）：全新会话直接回忆出上周的根因、commit 和待办 TODO，并附记忆引用，无需重新解释。*

## 你会得到什么

- Claude Code 和 Codex CLI 能跨会话记住项目决策。
- Bug 修复原因、偏好和项目模式可以搜索。
- 记忆默认留在本地：SQLite + SQLCipher。
- hooks、MCP tools、CLI 命令和 localhost REST API 共用同一份记忆库。
- current-memory contract 会暴露 staleness、temporal/as-of truth、
  citation usage 和 injection audit，不把召回过程当黑箱。
- user-context 控制把个人 claim、profile summary、suppression feedback
  和 Markdown export 保持为显式、可审阅的流程。
- 一个 Rust 二进制程序，不需要托管数据库或额外记忆服务。

## 安装

安装 `remem` 二进制：

```bash
brew install majiayu000/tap/remem
```

然后为本机已安装的 coding agents 配置 hooks 和 MCP：

```bash
REMEM_INSTALL_BINARY="$(brew --prefix remem)/bin/remem" remem install --target codex
# 或：REMEM_INSTALL_BINARY="$(brew --prefix remem)/bin/remem" remem install --target claude
# 或：REMEM_INSTALL_BINARY="$(brew --prefix remem)/bin/remem" remem install --target all
```

如果不用 Homebrew：

```bash
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | env REMEM_NO_CONFIG=1 sh
~/.local/bin/remem install --target codex
# 或：~/.local/bin/remem install --target claude
# 或：~/.local/bin/remem install --target all
```

`remem install` 可以自动检测已有的 Claude Code 和 Codex CLI 配置目录。
首次安装时请使用 `--target codex`、`--target claude` 或 `--target all`，
这样 remem 可以创建所选配置文件。

需要验证或排障时再运行 `remem doctor`。

## 成功检查

![remem install 与 SessionStart 上下文注入](assets/remem-demo.gif)

*`remem install` 配置了什么，以及新 Codex 会话在 SessionStart 收到的内容。demo 使用临时 HOME 和临时数据库，不会展示任何私人记忆。*

安装后启动一个新的 Claude Code 或 Codex CLI 会话。remem 应该在
SessionStart 时注入相关项目记忆，并在会话结束后总结耐久记忆。然后运行：

```bash
remem status
remem search "last decision"
```

对于 Codex CLI，`remem install` 会创建或更新：

- `~/.remem/.key` 和加密的 `~/.remem/remem.db`
- `~/.remem/config.toml` memory-AI profiles
- `~/.codex/config.toml` 中的 Codex MCP 注册
- `~/.codex/hooks.json` 中的 Codex SessionStart/Stop hooks

Codex-only setup 下，`remem doctor` 会把 Schema、Key format、Database 和
Codex Hooks/MCP 行报告为 ok。如果本机已经有 Claude Code 配置目录，但安装时
没有被自动检测到，再运行 `remem install --target claude` 或
`remem install --target all`。如果 doctor 出现多个 `remem` 二进制的
install-path 警告，按输出里的 fix 提示处理，确保 hooks 使用预期的二进制。

## 让你的 coding agent 安装

把这段发给 Claude Code 或 Codex CLI：

> Install remem for this repository. Use the official README. Configure it for
> this agent, run `remem doctor`, verify that session memory is working, and
> summarize what was installed.

## Claude Code 和 Codex 已经有 memory，为什么还要 remem？

内置 memory 适合简短偏好和稳定项目指引。

remem 面向需要可搜索、可审计、项目级隔离、可恢复的工程记忆：

- 用 `remem search` 搜索过去的决策、Bug 修复和原因
- 用 `remem why` 检查某条记忆为什么被注入
- 用 SQLite 和 SQLCipher 把记忆留在本地
- 通过 MCP 和 REST API 接入 coding agents 与本地工具
- 追踪后台记忆任务的用量和成本
- 避免手工维护很长的 `MEMORY.md` 或 `CLAUDE.md`

### remem 在生态中的位置

以下快照基于
[记忆工具生态调研（2026-03）](docs/research/claude-memory-mcp-ecosystem-2026-03.md)，
各项目当前特性请以其上游文档为准。

| | remem | 内置 memory 文件 | claude-mem | mem0 / OpenMemory |
|---|---|---|---|---|
| 采集 | Hook 自动采集 + LLM 提炼 | 手工编辑 | Hook 自动采集 | 依赖 agent 调用保存工具 |
| 支持的 agent | Claude Code + Codex 共享同一存储 | 各工具各自维护 | Claude Code | 任意 MCP 客户端 |
| 存储 | 本地 SQLite，可选 SQLCipher 加密 | 纯文本文件 | SQLite + Chroma 向量库 | 向量库，托管平台或本地服务 |
| 检索 | FTS + 可选 embeddings，CLI / MCP / REST | 整体加载 | 分层向量检索 | 向量检索 |
| 运行时 | 单个 Rust binary | 无 | Node worker + 后台服务 | Python 服务 |
| 可审计性 | `remem why`、来源追溯、用量与成本追踪 | Git 历史 | 调研中未见文档 | 调研中未见文档 |

## remem 如何解决会话失忆

| 没有 remem | 使用 remem |
|---|---|
| “我们用 FTS5 trigram tokenizer...” 每次都说 | 自动从记忆注入 |
| “非测试代码不要 `expect()`” 反复提醒 | 偏好会优先展示 |
| “上次我们决定了什么...” 需要手动回溯 | 决策历史可追踪 |
| 会话结束就丢失修复背景 | 根因与修复被持续保留 |

## 其他安装渠道

```bash
# 快速安装选项
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | env REMEM_NO_CONFIG=1 REMEM_VERSION=vX.Y.Z sh
~/.local/bin/remem install --target codex

curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | env REMEM_NO_CONFIG=1 sh
~/.local/bin/remem install --target codex

curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | env REMEM_NO_CONFIG=1 REMEM_INSTALL_DIR=/usr/local/bin sh
remem install --target codex

# npm wrapper
npm install -g @remem-ai/remem
remem install --target codex

# Cargo
cargo install remem-ai --bin remem
remem install --target codex

# 手动下载 GitHub Release
curl -LO https://github.com/majiayu000/remem/releases/latest/download/remem-darwin-arm64.tar.gz
tar xzf remem-darwin-arm64.tar.gz
mv remem ~/.local/bin/
codesign -s - -f ~/.local/bin/remem  # macOS ARM 必须签名
~/.local/bin/remem install --target codex

# 源码构建
git clone https://github.com/majiayu000/remem.git
cd remem
cargo build --release
cp target/release/remem ~/.local/bin/
codesign -s - -f ~/.local/bin/remem  # macOS ARM 必须签名
~/.local/bin/remem install --target codex
```

PATH 上建议只保留一个 canonical `remem` 命令。Standalone 和源码安装通常放在
`~/.local/bin/remem`；Windows standalone 安装建议放在
`%USERPROFILE%\.local\bin\remem.exe`。如果使用 Homebrew 或 Cargo 这类包管理器，
后续也通过同一个渠道升级，避免 PATH 前后同时残留第二份手动安装的二进制。
`remem doctor` 和 `remem install --dry-run` 会在检测到多个 `remem` 可执行文件时警告。

### 更新已有安装

手动替换二进制后，需要重新执行 `remem install`，让已有 Claude Code 和
Codex hook 命令刷新到当前 host-aware 配置：

```bash
cargo build --release
cp target/release/remem ~/.local/bin/
codesign -s - -f ~/.local/bin/remem  # macOS ARM 必须签名
remem install --target all
```

验证已安装 hooks 是否包含 host-specific context 命令：

```bash
jq -r '.hooks.SessionStart[]?.hooks[]?.command' ~/.claude/settings.json
jq -r '.hooks.SessionStart[]?.hooks[]?.command' ~/.codex/hooks.json
```

期望看到 host-only command；模型、executor、上下文策略都在
`~/.remem/config.toml`：

```text
/Users/you/.local/bin/remem context --host claude-code
/Users/you/.local/bin/remem context --host codex-cli
```

## 在 Codex 中使用

`remem install --target codex` 会配置四类 Codex 集成：

- 在 `~/.codex/config.toml` 中启用 `[features].hooks = true`
- 在 `~/.codex/config.toml` 中注册 `remem` MCP server
- 在 `~/.codex/hooks.json` 中写入 Codex hook 命令
- 创建或更新 `~/.remem/config.toml` memory-AI profiles

重启 Codex 后，remem 会在 session start 自动注入相关项目记忆，并在 stop
时后台总结本次会话。Codex 也可以调用 `remem mcp` 暴露的 MCP 工具，包括
`search`、`get_observations`、`save_memory`、`workstreams` 和 `timeline`。

默认 Codex 集成刻意保持低噪音：用 `SessionStart` 注入上下文，用 `Stop`
做后台总结。Codex 通过 `[memory_ai.hosts."codex-cli"].context_gate = "strict"`
启用 strict duplicate-injection gate，所以同一 session 里中途重复触发的
`SessionStart` 会在首次注入后保持静默；默认不安装高频 Bash observe hook。

## 发布渠道

当前已发布：

- Homebrew：`brew install majiayu000/tap/remem`
- GitHub Releases：macOS/Linux 的 x64/arm64 预编译二进制
- crates.io：`cargo install remem-ai --bin remem`
- npm：`npm install -g @remem-ai/remem`
- 源码构建：`cargo build --release`

适合继续扩展的渠道：

- apt/yum packages：等 Linux 的安装路径和服务管理方案稳定后再做

## 工作机制

`remem` 使用 host-specific hook 策略自动运行：

```
Claude Code 正常工作流
        |
        |- SessionStart      -> 注入记忆与偏好
        |- UserPromptSubmit  -> 注册会话、刷新旧队列
        |- PostToolUse       -> 捕获工具操作（入队，<1ms）
        '- Stop              -> 后台总结（返回约 6ms）

Codex 正常工作流
        |
        |- SessionStart      -> 注入记忆与偏好
        '- Stop              -> 使用 Codex CLI 后台总结
```

Codex 默认不再安装高频 `PostToolUse(Bash)` observe hook。Shell-heavy 会话必须等 coalesced capture 管线接管后再开启逐命令捕获，否则 Bash 输出会制造无界 backlog。旧 hook 即使残留，也会被默认忽略；只有显式设置 `REMEM_ENABLE_CODEX_BASH_OBSERVE=1` 才会重新开启。

capture 管线从 append-only ledger 开始：`captured_events` 保存原始 hook/session evidence，`event_blobs` 承接大 payload，`extraction_tasks` 按 host/project/session 合并后台提炼任务，避免每个工具调用都生成一个 LLM job。长期记忆仍然是这条管线 promotion 后的结果，不是 raw event 本身。

## remem 与内置 `MEMORY.md`

当上下文很小、很稳定、并且值得手工维护时，内置 memory file 就够用：项目规则、安装说明、少量长期偏好，都适合放在一眼能看到的文件里。

remem 解决的是不应该依赖手工维护的部分：

- **自动捕获与召回**：hooks 会把会话总结进 SQLite 记忆库；`remem search`、`remem show`、`timeline` 和 MCP `get_observations` 可以按需取回详细内容。
- **与原生 memory 的桥接**：当 Claude Code native memory 目录存在时，`remem sync-memory --cwd .` 会写入 compact 的 `remem_sessions.md`，并在 `MEMORY.md` 中追加指针和大小保护。完整细节仍保留在数据库里，用 `remem search` 查询。
- **可人工编辑的 Markdown 镜像**：`remem export --markdown --output ./remem-memory --project "$PWD"` 会把每条 curated memory 写成一个 `.md` 文件，且目标目录必须为空。编辑这些文件后，`remem import markdown --source ./remem-memory` 会更新已有行，并重建 search、entity、embedding 和 current-state 索引。导出会拒绝非空目录，避免覆盖人工编辑。
- **治理与可审计性**：`remem why <id>`、`remem govern --action stale --dry-run --json <id>`、`remem status --json`、`remem usage --days 14 --weeks 8` 分别用于查看记忆为什么可见、预览治理操作、检查存储健康，以及查看 memory-AI token/费用统计。
- **current-memory 可解释性**：staleness label、temporal fact、source-anchor check、injection item audit row、citation/usage event 会说明一条记忆为什么 current、stale、dropped、abstained、cited 或 ignored。
- **user-context 治理**：`remem user ...`、`remem memory suppress ...`、profile export 和 non-retention policy check 让个性化召回保持显式，而不是把所有用户事实静默混入所有项目。
- **声明前的确定性检查**：本地门禁包括 `cargo test -q context::claude_memory --lib`、`cargo test -q eval::golden --lib`、`cargo test -q eval::governance --lib` 和 `remem eval-e2e --json`。

这不是“remem 已经在真实编码任务上击败精心维护的 `MEMORY.md`”的公开 benchmark 声明。no-memory / remem / curated-file 三组旗舰 A/B 仍是单独的 benchmark 要求；在它发布前，诚实边界是功能覆盖和可复现的本地检查。

## 检索架构

`remem` 使用受 [Hindsight](https://github.com/vectorize-io/hindsight) 启发的 4 通道 RRF 融合检索：

```
Query: "database encryption"
        |
   +----+------------------------------------+
   |            4 个并行通道                 |
   +-----------------------------------------+
   | 1. FTS5 (BM25)   trigram + OR           |
   | 2. Entity Index  1600+ 实体             |
   | 3. Temporal      "yesterday"/"last week" |
   | 4. LIKE fallback 短 token 回退          |
   +-------------+---------------------------+
                 |
        RRF score = sum(1 / (60 + rank_i))
                 |
              输出融合后的 Top-K
```

增强能力：

- 实体图扩展（2-hop 多跳检索）
- 项目级隔离检索（避免跨项目污染）
- CJK 分词支持
- 中英文同义词扩展
- 标题加权 BM25（`bm25(fts, 10.0, 1.0)`）
- 基于内容哈希的 `topic_key` 去重
- MCP 工具中的多步检索引导

## 基准概览

### Public artifact suite（仅方向性证据）

仓库内的 `eval/public` artifact 会把 memory-system capability evidence
和 coding-agent outcome evidence 分开。可以用下面的命令复现 public verifier：

```bash
cargo run -- bench verify --root eval/public --json-out /tmp/remem-bench-verify.json
cargo run -- bench report --root eval/public --json-out eval/public/reports/baseline.json --markdown-out eval/public/reports/baseline.md
```

当前 directional report 会验证 4 个 manifest、4 个 report、25 个 run
artifact 和 125 个 artifact 文件。它包含：

- `remem-code-memory`：8 个 memory QA run，覆盖 temporal/as-of 回答、stale
  decision avoidance、conflict、workstream continuity、prior bug root cause、
  architecture constraint、file/source anchor 和 user-context relevance。
- `adversarial-policy`：15 个 non-retention case，覆盖 secrets、credentials、
  payment data、unsupported assistant claim、unapproved external source、
  roleplay、negation、same-name repo、branch divergence、stale file anchor
  和 unresolved conflict。
- `issue385-smoke`：一个已提交的 coding-agent smoke run artifact，并为
  `remem` run 记录 memory-contract 字段。完整 `issue385-v1` fixture pack
  目前只作为 dry-run 复现输入引用，还不属于已验证的 public outcome report。

该报告有意标记为 `directional_only_no_public_claim`。README 和 release
文案只能保持方向性，不发布宽泛效果胜出或编码任务效果胜出声明，直到
[`docs/release-lifecycle.md`](docs/release-lifecycle.md) 中的 public claim
gate 通过。

### 隔离 coding-agent baseline（内部证据，不是公开 claim）

`eval/coding-bench/reports/baseline.json` 包含一个隔离的 5-task、
每个 condition 3 次运行的 baseline，运行器是 `codex-cli 0.142.1`，
模型是 `gpt-5.5`：`no_memory` 解决 2/15，`remem` 解决 15/15，
`curated_file` 解决 15/15。这是有用的工程证据，但它早于公开 16-task
v1 fixture pack，必须重新生成后才能支持更强的产品声明。

### LoCoMo（仅作信息参考）

完整 [LoCoMo](https://github.com/snap-research/locomo) 基准（10 个会话，对抗类跳过后共 1540 QA）：

这个快照只是历史脚注，不作为 CI 或发布门禁。确定性门禁使用 golden
retrieval eval；LoCoMo 仅保留给人工信息参考，因为该基准的方法论已有争议。

| 配置 | Overall | Single-hop | Multi-hop | Temporal | Open-domain | Ingest | Model |
|---|---:|---:|---:|---:|---:|---|---|
| **v1（公平对比）** | **56.8%** | 67.1% | 39.0% | 53.9% | 28.1% | per-turn | gpt-5.4 |
| **v2（优化）** | **62.7%** | 72.3% | 61.3% | 40.5% | 56.2% | session_summary | gpt-5.4 |

### 内部评测（1777 条真实记忆）

| 指标 | 数值 |
|---|---:|
| MRR | 0.858 |
| Hit Rate@5 | 1.000 |
| Dedup rate | 1.0% |
| Project leak | 0% |
| Self-retrieval | 100% |

### 本地端到端 QA 评测

```bash
python3 eval/local/run_local_eval.py --db ~/.remem/remem.db --n 20
```

| 指标 | 分数 |
|---|---:|
| Overall | **85.0%** |
| Decision | 77.8% |
| Discovery | 87.5% |
| Preference | 100% |
| Source in top-20 | 90.0% |

需要显式传入 `--db`，并在项目根目录 `.env` 中配置 `OPENAI_API_KEY`（可选 `OPENAI_BASE_URL`、`OPENAI_MODEL`）。

## Token 使用量与费用统计

remem 会为自己的后台提炼、总结、压缩、promotion 调用写入 AI usage
账本。CLI 可以查看每日、每周 token 使用量和估算费用：

```bash
remem usage --days 14 --weeks 8
remem usage --project /path/to/project --days 30 --weeks 12
```

报表包含调用次数、input/cache/output/reasoning token、total token、估算
美元费用，以及统计精度说明。usage row 会标记来源：

- `anthropic_usage`：Anthropic Messages API 返回的 provider usage
- `codex_log`：从当前 `codex exec --json` 的 `turn.completed.usage`
  事件解析出的精确 token count
- `text_estimate`：拿不到真实 usage 时，用 prompt/response 文本长度估算

费用是估算，不是账单。历史数据可能是文本估算，也可能从旧的无精确模型记录
重估过。

## Memory AI 配置

Memory AI 执行策略配置在 `~/.remem/config.toml`（可用 `REMEM_CONFIG`
覆盖路径）。hooks 只传 `--host`；config 负责把每个 host 映射到一个 profile，
统一用于 summarize、flush/extract、compress 和 dream。

```bash
remem config path
remem config show
remem config set memory_ai.profiles.codex.model gpt-5.2
```

日常切模型推荐用更高层的 `remem model` 命令：

```bash
remem model current
remem model list
remem model use cheap
remem model use balanced --dry-run
remem model use gpt-5.2 --reasoning medium
remem model use haiku --host claude-code
remem model test
remem model test --live
remem model rollback
```

`remem model test` 默认只校验配置，不产生 AI 调用成本；只有加 `--live`
才会实际调用模型。`remem model use` 写入配置前会保存 rollback 备份。内置
preset 主要面向 Codex；Claude Code profile 建议直接传明确模型名。

默认 Codex profile：

```toml
[memory_ai.hosts."codex-cli"]
memory_profile = "codex"
context_gate = "strict"
context_color = true
capture_adapter = "codex-cli"

[memory_ai.profiles.codex]
executor = "codex-cli"
model = "gpt-5.2"
path = "codex"
```

## 常用命令

```bash
remem install
remem uninstall
remem doctor
remem search "query"
remem search "query" --branch main --type decision --multi-hop --offset 10
remem show <id>
remem eval
remem eval-local
remem backfill-entities
remem encrypt
remem api --port 5567
remem status
remem config show
remem config set memory_ai.profiles.codex.model gpt-5.2
remem model current
remem model list
remem model use balanced --dry-run
remem model use gpt-5.2 --reasoning medium
remem model use haiku --host claude-code
remem model test [--live]
remem model rollback
remem usage --days 14 --weeks 8
remem pending list-failed
remem pending retry-failed --dry-run
remem pending purge-failed --dry-run --older-than-days 7
remem review list
remem review approve <id>
remem review discard <id>
remem review edit <id> --text "updated memory"
remem preferences list
remem preferences add "text"
remem preferences remove 42
remem context --cwd .
remem cleanup --dry-run --json
remem cleanup
remem dream [--project X] [--profile NAME] [--dry-run]
remem install --target codex
remem mcp
remem sync-memory --cwd .
```

## REST API

```bash
remem api --port 5567
TOKEN=$(cat ~/.remem/.api-token)
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:5567/api/v1/status
```

直接使用库 API 构建 router 的调用方，应在 `remem::api::build_router(...)`
之前调用 `remem::api::ensure_api_token()`。

| Endpoint | Method | 说明 |
|---|---|---|
| `/api/v1/search?query=&project=&type=&limit=&offset=&branch=&multi_hop=` | GET | 检索记忆 |
| `/api/v1/memory?id=` | GET | 获取单条记忆 |
| `/api/v1/memories` | POST | 保存记忆 |
| `/api/v1/status` | GET | 系统状态 |

## 安全性

- SQLCipher 静态加密（`remem encrypt`）
- 数据目录权限（`0700`）
- 密钥文件权限（`0600`）
- REST API 仅绑定本机（`127.0.0.1`），并要求
  `Authorization: Bearer $(cat ~/.remem/.api-token)`
- API token 文件权限（`0600`）

## 架构文档

详细设计见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

## 卸载

```bash
remem uninstall
rm -rf ~/.remem
```

## License

MIT
