# remem

> 不用在每次新会话里重复解释你的项目。

语言： [English](README.md) | **简体中文**

`remem` 是面向 Claude Code 的持久记忆工具。它是一个 Rust 单二进制程序，会在会话间自动捕获、提炼并注入项目上下文，包括决策、模式、偏好和经验。

[![CI](https://github.com/majiayu000/remem/actions/workflows/ci.yml/badge.svg)](https://github.com/majiayu000/remem/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## 核心问题

- **会话失忆**：每次新开 Claude Code 会话都要从零解释。
- **上下文丢失**：Bug 修复原因、设计取舍会在会话结束后消失。
- **偏好重复**：同样的编码偏好要反复强调。
- **缺少连续性**：跨会话推进功能时恢复成本高。

## remem 的解决方式

| 没有 remem | 使用 remem |
|---|---|
| “我们用 FTS5 trigram tokenizer...” 每次都说 | 自动从记忆注入 |
| “非测试代码不要 `expect()`” 反复提醒 | 偏好会优先展示 |
| “上次我们决定了什么...” 需要手动回溯 | 决策历史可追踪 |
| 会话结束就丢失修复背景 | 根因与修复被持续保留 |

## 安装

```bash
# 方式 1：快速安装（预编译二进制）
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | sh

# 方式 2：Cargo 安装
cargo install remem

# 方式 3：源码构建
git clone https://github.com/majiayu000/remem.git
cd remem
cargo build --release
cp target/release/remem ~/.local/bin/
codesign -s - -f ~/.local/bin/remem  # macOS ARM 必须签名

# 配置 Claude Code hooks + MCP
remem install
```

安装后重启 Claude Code。

## 工作机制

`remem` 通过 Claude Code Hooks 自动运行：

```
你的 Claude Code 正常工作流
        |
        |- SessionStart      -> 注入记忆与偏好
        |- UserPromptSubmit  -> 注册会话、刷新旧队列
        |- PostToolUse       -> 捕获工具操作（入队，<1ms）
        '- Stop              -> 后台总结（返回约 6ms）
```

全程自动，不需要手动保存记忆。

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

### LoCoMo

完整 [LoCoMo](https://github.com/snap-research/locomo) 基准（10 个会话，对抗类跳过后共 1540 QA）：

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
python3 eval/local/run_local_eval.py --n 20
```

| 指标 | 分数 |
|---|---:|
| Overall | **85.0%** |
| Decision | 77.8% |
| Discovery | 87.5% |
| Preference | 100% |
| Source in top-20 | 90.0% |

需要项目根目录 `.env` 中配置 `OPENAI_API_KEY`（可选 `OPENAI_BASE_URL`、`OPENAI_MODEL`）。

## 常用命令

```bash
remem install
remem uninstall
remem doctor
remem search "query"
remem show <id>
remem eval
remem eval-local
remem backfill-entities
remem encrypt
remem api --port 5567
remem status
remem pending list-failed
remem pending retry-failed
remem pending purge-failed
remem preferences list
remem preferences add "text"
remem preferences remove 42
remem context --cwd .
remem cleanup
remem mcp
remem sync-memory --cwd .
```

## REST API

```bash
remem api --port 5567
```

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
- API 仅绑定本机（`127.0.0.1`）

## 架构文档

详细设计见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

## 卸载

```bash
remem uninstall
rm -rf ~/.remem
```

## License

MIT
