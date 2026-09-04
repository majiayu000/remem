# remem｜Claude Code 与 Codex 的本地优先记忆系统

> 新开一个 coding-agent 会话，不用再从头解释项目。

语言　[English](README.md) | **简体中文**

`remem` 会自动捕获、提炼、搜索并注入 Claude Code 和 OpenAI Codex CLI 的
工程记忆。项目决策、Bug 根因、开发模式和偏好通过 hooks、MCP、
CLI 与本机 REST API 持续回到后续会话。

[![CI](https://github.com/majiayu000/remem/actions/workflows/ci.yml/badge.svg)](https://github.com/majiayu000/remem/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/majiayu000/remem?sort=semver)](https://github.com/majiayu000/remem/releases/latest)
[![crates.io](https://img.shields.io/crates/v/remem-ai)](https://crates.io/crates/remem-ai)
[![npm](https://img.shields.io/npm/v/%40remem-ai%2Fremem)](https://www.npmjs.com/package/@remem-ai/remem)
[![License MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

![Remem 记忆接续演示](assets/remem-recall-demo.gif)

*一个新的 Claude Code 会话直接找回之前的根因、commit 和待办，并附上记忆引用。*

## remem 能带来什么

- 自动捕获会话，并在后台用 LLM 提炼长期记忆。
- Claude Code 与 Codex 共用一份项目级本地记忆。
- 决策、Bug 修复、架构说明、偏好和原始会话证据都可以搜索。
- 每条记忆带有来源、时效状态、审核和注入记录。
- 新安装默认使用 SQLite 与 SQLCipher 加密。
- MCP、CLI 和带认证的本机 REST API 共用同一套 Rust runtime。

remem 优先保证记忆质量。自动 hook 捕获承担主要工作，手动
`save_memory` 用于及时补充重要决策。

## 五分钟安装

### Homebrew

```bash
brew install majiayu000/tap/remem
"$(brew --prefix remem)/bin/remem" install --target codex
```

Claude Code 使用 `--target claude`。`--target all` 会配置所有已知 host，
其中也包括平台支持时的 Cursor v1。

### 独立安装脚本

```bash
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | env REMEM_NO_CONFIG=1 sh
~/.local/bin/remem install --target codex
```

### npm 或 Cargo

```bash
npm install -g @remem-ai/remem
# 或
cargo install remem-ai --bin remem

remem install --target codex
```

GitHub Release 也提供带校验和的 macOS、Linux x64/arm64 二进制。系统里尽量
只保留一个 `remem`。hooks 和终端若解析到不同副本，`remem doctor` 会给出警告。

各安装渠道的升级方法、平台边界、PATH 漂移和手动安装说明见
[安装与升级指南](docs/installation.md)。插件和运维材料可以继续从
[文档导航](docs/README.md)进入。

## 验证安装

重启刚配置的 coding agent，然后运行下面的命令。

```bash
remem doctor
remem status
remem search "last decision"
```

Claude Code 或 Codex 安装正常时，remem 会在 SessionStart 注入相关项目记忆，
并在 Stop 后排队提炼本次会话。`remem doctor` 会检查 schema、加密密钥、数据库、
hooks、MCP 注册、worker 和常见的安装路径漂移。

仓库贡献者可运行[隔离的可执行 smoke fixture](scripts/ci/smoke_sessionstart_context_gate.sh)，
验证重复 SessionStart 注入抑制。

下面的命令可以只读检查当前记忆状态。

```bash
remem doctor truth --cwd .
```

## Host 支持范围

| 能力 | Claude Code | Codex CLI | Cursor v1 |
|---|---|---|---|
| MCP 记忆工具 | 支持 | 支持 | macOS/Linux 支持 |
| SessionStart 注入 | 支持 | 支持 | 不支持 |
| 自动会话记忆 | 支持 | 支持，基于 Stop，默认低噪音 | v1 安装器不会启用 |
| 工具事件捕获 | 安装 hooks | 默认没有高频 Bash hook | runtime 命令存在，但不安装 hook |
| 编译后的命令规则 | Bash 可选 warn/block | 不支持 | 不支持 |
| Windows | 支持 | 支持 | 不支持 |

Cursor v1 安装器只注册 MCP。已经验证的 `observe` 和 `summarize` runtime
命令可以使用，但 `remem install --target cursor` 不会安装自动捕获 hook，也没有
SessionStart 注入。

仓库还带有 Codex plugin wrapper。安装本地插件 runtime 和显式启用 hooks 的方法
见 [plugins/remem/README.md](plugins/remem/README.md)。

## 为什么还要保留内置 memory 文件

`MEMORY.md`、`CLAUDE.md` 和 agent instruction 文件适合少量、稳定、每次都应该
看到的规则。remem 负责数量更大、变化更快、需要来源证据的工程历史。

| 使用场景 | 内置文件 | remem |
|---|---|---|
| 稳定项目规则 | 很适合 | 支持 |
| 自动捕获会话 | 需要手工维护 | hooks 自动处理 |
| 搜索旧决策的原因 | 受加载文本限制 | 支持 curated 与 raw search |
| 分支、时间和过期状态 | 手工处理 | 内置支持 |
| 来源与注入审计 | 依赖 Git 历史 | 数据库审计 |
| 审核、抑制与生命周期治理 | 手工编辑 | 有专门命令 |

两者可以同时使用。简短规则留在原生文件里，决策、失败证据和持续变化的项目
状态交给 remem。

更广泛的同类工具对比保留在带日期的
[memory 工具调研](docs/research/claude-memory-mcp-ecosystem-2026-03.md)中。

## 工作方式

```text
Claude Code / Codex hooks
          |
          v
append-only captured_events ledger
          |
          v
合并后的后台 extraction 与 session rollup
          |
          v
受治理的 candidate -> curated memory + workstream + raw archive
          |
          v
FTS、entity、temporal、vector、graph 与可选本地 reranker
          |
          v
有预算、有来源的 SessionStart context
```

hooks 完成可靠捕获或入队后就会返回。后台 worker 负责提炼、candidate 治理、
压缩、检索增强和生命周期清理。MCP、CLI、REST 与 SessionStart 使用同一份本地
存储和治理模型，但各自应用不同的 eligibility policy。显式检索是检查与恢复
surface，因此可能返回带 `legacy_unverified` 标签的 memory；默认 SessionStart 与
CurrentTruth 会隔离这些记录，并留下原因。

模型生成的内容要经过来源支持、secret、instruction pattern、scope 和生命周期
检查。没有通过的内容会被丢弃或进入 review，并留下可以诊断的原因。

模块边界和当前数据流见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

实验性的 MCP `context_bundle` 向显式调用者开放带版本和预算的 compiler。
实验性的 `remem context-plan` 可以输出一次请求对应的 retrieval plan。两个
opt-in 接口分别由 [Context Bundle](docs/specs/GH932/PRODUCT.md)和
[retrieval router](docs/specs/GH934/PRODUCT.md)契约跟踪。

## 常用流程

### 检索与查看

```bash
remem search "database encryption"
remem search "deployment decision" --branch main --explain
remem show <memory-id>
remem why <memory-id>
remem current <state-key>
```

agent 通过 MCP `search` 取得简短结果，再用 `get_observations` 读取选中的细节。
curated memory 没有找到精确对话证据时，再查询 raw archive。

```bash
remem raw search "exact phrase" --since 2026-06-01 --json
```

读取精确会话前，先列出带 host 绑定的完整会话：

```bash
remem raw sessions --latest 20 --json
remem raw messages --host codex-cli --source-root local \
  --project "/path/to/project" --session-id SESSION_ID --json
remem ingest-sessions --root codex-cli:archive=/path/to/sessions --json
```

把同一条 `raw sessions` 摘要中的 `host`、`source_root`、`project` 和
`session_id` 原样传给 `raw messages`。旧脚本必须补上必填的 `--host`，并把
`--root LABEL=PATH` 改成 `--root HOST:LABEL=PATH`；`raw reconcile` 使用相同
格式。`HOST` 只能是 `claude-code` 或 `codex-cli`，`LABEL` 会持久化为
`source_root`。Cursor snapshot 证据需要手动配置并验证
`remem summarize --host cursor` Stop 集成；文件系统 `--root` 摄取和对账会
明确拒绝 `cursor`。

### 审核与治理

```bash
remem review list
remem review approve <candidate-id>
remem memory suppress memory:<id> --reason "no longer relevant"
remem govern --action stale --dry-run --json <id>
```

写入型治理命令会按风险提供预览、明确确认或 review 边界。最新参数以
`remem <command> --help` 为准，README 不再手工复制整份命令表。

MCP 工具使用同一份本地存储，但有更严格的线契约。规范入口是
[GH981](docs/specs/GH981/PRODUCT.md)，并包含 #1061 的变更与范围边界：

- `save_memory`：已知调用 host 时传入 `host`。省略时记为 `unknown`，不会推断成
  `codex-cli`。
- `govern_memory`：先 `dry_run`，预览 ID 以及该治理事务里加载到的当前版本。
  正式写入必须为每个 ID 提供 `expected_versions`，并带上
  `confirm_destructive=true` 和明确 reason。
- `recall_user_context`：必须提供 `project` 或 `cwd`。服务器不会用自身进程的
  工作目录推断范围。

### 配置 Memory AI 与检索

```bash
remem config show
remem model current
remem model use balanced --dry-run
remem embedding status
remem embedding download --model multilingual-e5-small
remem embedding backfill --limit 1000
```

`auto` embedding 不会因为环境里碰巧存在 `OPENAI_API_KEY` 就发起远程请求。
经过验证的本地模型可以选择安装，也可以继续使用带明确标签的 feature-hash
fallback。第二阶段本地 reranker 同样是可选能力，配置前保持关闭。

详细说明见[当前配置入口](docs/README.md#configuration)、
[本地 embedding 契约](docs/specs/local-semantic-embedding/PRODUCT.md)，以及
`remem config`、`remem embedding`、`remem reranker` 的命令帮助。

### 在数据库之外查看或共享记忆

<!-- remem-doc-contract:current-project-export:start -->
```bash
remem sync-memory --cwd .
remem export --markdown --output ./remem-memory
remem export --pack .remem-pack
```
<!-- remem-doc-contract:current-project-export:end -->

Markdown mirror 可以人工编辑。Project memory pack 是确定性、可提交到 Git 的
导出格式，import 会处理来源、冲突和 quarantine。继续阅读
[memory 使用指南](docs/memory-usage-guide.md)和
[project memory pack 契约](docs/specs/project-memory-pack/PRODUCT.md)。

## 证据与评测

仓库内的 public suite 分开记录 memory-system capability evidence 和
coding-agent outcome evidence。可以运行下面的验证命令。

```bash
cargo run -- bench verify --root eval/public --json-out /tmp/remem-bench-verify.json
```

当前 public report 不能用于对外 benchmark 声明，并明确标记为
`directional_only_no_public_claim`。历史隔离
coding baseline 可以用于工程分析，但其中的预载 memory condition 不能和当前
SessionStart 检索路径直接比较。

复现命令、artifact schema、claim 边界和门禁由下面几个页面维护。

- [eval/README.md](eval/README.md)
- [eval/public/README.md](eval/public/README.md)
- [eval/coding-bench/README.md](eval/coding-bench/README.md)

没有 checked-in report 的本地数字不再放进 README。

## 安全与隐私

- 新安装会创建 SQLCipher 加密数据库和独立密钥文件。
- 数据目录和密钥使用严格的用户级权限。
- REST API 只绑定 `127.0.0.1`，并要求 bearer token。
- hook 捕获的事件预览会在持久化之前清理敏感内容。
- candidate 和注入内容都要经过 secret 与 poisoning defense。
- `remem doctor` 可以报告加密、明文残留、schema 与审计故障，不打印记忆正文。

安全报告方式和政策见 [SECURITY.md](SECURITY.md)。
[SQLite tuning](docs/specs/GH949/PRODUCT.md)与
[memory poisoning defense](docs/specs/memory-poisoning-defense/PRODUCT.md)的运维契约
放在独立文档中维护。

## REST API

```bash
remem api --port 5567
TOKEN=$(cat ~/.remem/.api-token)
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:5567/api/v1/health
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:5567/api/v1/capabilities
```

client 应通过 `/api/v1/capabilities` 判断能力。当前 endpoint 与兼容性契约见
[docs/specs/SPEC-web-api.md](docs/specs/SPEC-web-api.md)。

## 文档导航

[docs/README.md](docs/README.md) 是安装、配置、记忆生命周期、检索、治理、
API、插件、运维、架构和评测材料的跳转页面。

下面列出常用入口。

- [架构与数据流](docs/ARCHITECTURE.md)
- [Memory 使用指南](docs/memory-usage-guide.md)
- [Memory 生命周期](docs/memory-lifecycle.md)
- [Codex plugin](plugins/remem/README.md)
- [REST API 契约](docs/specs/SPEC-web-api.md)
- [当前 spec 索引](docs/specs/README.md)
- [Changelog](CHANGELOG.md)
- [参与贡献](CONTRIBUTING.md)

## 卸载

先预览，再移除 host hooks 和 MCP 注册。这个过程不会删除记忆。

```bash
remem uninstall --dry-run
remem uninstall
```

加密数据库会保留在配置的 `REMEM_DATA_DIR`。确实需要移除本机数据时，先备份，
再人工删除该目录。普通文件删除只能移除 remem 的本地数据，并不保证从文件系统
快照、备份或底层存储介质中安全擦除这些数据。

## License

MIT
