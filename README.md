# Remem Memory

> Automatic memory for Claude Code and Codex.

Language: **English** | [简体中文](README.zh-CN.md)

`remem` is a single Rust binary that automatically captures, distills, and injects project context across Claude Code and Codex sessions: decisions, patterns, preferences, and learnings. Stop re-explaining your project every new session.

[![CI](https://github.com/majiayu000/remem/actions/workflows/ci.yml/badge.svg)](https://github.com/majiayu000/remem/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

![Remem Memory terminal demo](assets/remem-demo.gif)

## The Problem

- **Session amnesia**: every new Claude Code or Codex session starts from zero.
- **Lost context**: bug-fix rationale and design decisions disappear after the session ends.
- **Preference fatigue**: the same preferences must be repeated every session.
- **No continuity**: long-running work is hard to resume with confidence.

## How remem Solves This

| Without remem | With remem |
|---|---|
| "We use FTS5 trigram tokenizer..." (every session) | Injected automatically from memory |
| "Do not use `expect()` in non-test code" (again) | Preference surfaced before you ask |
| "Last session we decided to..." (reconstruct manually) | Decision history with rationale |
| Bug context lost after session ends | Root cause + fix preserved |

## Install

```bash
# Option 1: Homebrew
brew install majiayu000/tap/remem

# Option 2: Quick install (prebuilt GitHub Release binary)
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | sh

# Pin a specific release or install into a custom bin directory
REMEM_VERSION=v0.4.5 curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | sh
REMEM_INSTALL_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | sh
REMEM_NO_CONFIG=1 curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | sh

# Option 3: Manual GitHub Release download
curl -LO https://github.com/majiayu000/remem/releases/latest/download/remem-darwin-arm64.tar.gz
tar xzf remem-darwin-arm64.tar.gz
mv remem ~/.local/bin/
codesign -s - -f ~/.local/bin/remem  # required on macOS ARM

# Option 4: Cargo
cargo install remem-ai --bin remem

# Option 5: Build from source
git clone https://github.com/majiayu000/remem.git
cd remem
cargo build --release
cp target/release/remem ~/.local/bin/
codesign -s - -f ~/.local/bin/remem  # required on macOS ARM

# Configure detected Claude Code/Codex hooks + MCP
remem install

# Optional: target one host explicitly
remem install --target codex    # auto | claude | codex | all
remem install --dry-run         # preview config changes
```

Restart your AI coding tool after installation.

## Use With Codex

For Codex-only setup:

```bash
remem install --target codex
remem doctor
remem status
```

`remem install --target codex` configures Codex in three ways:

- Enables Codex hooks with `[features].hooks = true` in `~/.codex/config.toml`
- Registers `remem` as an MCP server in `~/.codex/config.toml`
- Writes Codex hook commands to `~/.codex/hooks.json`

After restarting Codex, remem automatically injects relevant project memory at
session start and summarizes the session at stop. Codex can also call the MCP
tools exposed by `remem mcp`, including `search`, `get_observations`,
`save_memory`, `workstreams`, and `timeline`.

The default Codex integration is intentionally low-noise: it uses
`SessionStart` for context injection and `Stop` for background summarization.
It does not install high-frequency Bash observation by default.

## Distribution Channels

Currently published:

- Homebrew: `brew install majiayu000/tap/remem`
- GitHub Releases: prebuilt binaries for macOS and Linux on x64/arm64
- crates.io: `cargo install remem-ai --bin remem`
- Source build: `cargo build --release`

Prepared but not published:

- npm wrapper package in `npm/remem`; publish requires npm auth or an
  `NPM_TOKEN` secret

Good next channels:

- apt/yum packages: useful later, after the binary install path and service
  story are stable across Linux distributions

## How It Works

remem uses host-specific hook strategies:

```
Claude Code workflow
        |
        |- SessionStart      -> Inject memories + preferences
        |- UserPromptSubmit  -> Register session, flush stale queues
        |- PostToolUse       -> Capture tool operations (queued, <1ms)
        '- Stop              -> Summarize in background (~6ms return)

Codex workflow
        |
        |- SessionStart      -> Inject memories + preferences
        '- Stop              -> Summarize in background with Codex CLI
```

Codex does not install a high-frequency `PostToolUse(Bash)` observe hook by
default. Shell-heavy sessions must use the coalesced capture pipeline before
per-command capture is enabled again; otherwise Bash output can create an
unbounded backlog. Existing legacy hooks are also ignored unless
`REMEM_ENABLE_CODEX_BASH_OBSERVE=1` is set explicitly.

The capture pipeline starts with an append-only ledger:
`captured_events` stores raw hook/session evidence, `event_blobs` keeps large
payloads out of prompt-sized rows, and `extraction_tasks` coalesces work by
host/project/session instead of creating one LLM job per tool call. Curated
memory remains the promoted output of this pipeline, not the raw event itself.

## Search Architecture

remem uses 4-channel Reciprocal Rank Fusion (RRF) inspired by [Hindsight](https://github.com/vectorize-io/hindsight):

```
Query: "database encryption"
        |
   +----+------------------------------------+
   |          4 parallel channels            |
   +-----------------------------------------+
   | 1. FTS5 (BM25)   trigram + OR           |
   | 2. Entity Index  1600+ entities         |
   | 3. Temporal      "yesterday"/"last week" |
   | 4. LIKE fallback short tokens           |
   +-------------+---------------------------+
                 |
        RRF score = sum(1 / (60 + rank_i))
                 |
             Top-K merged results
```

Enhancements:

- Entity graph expansion (2-hop multi-hop retrieval)
- Project-scoped entity search (no cross-project leakage)
- CJK segmentation support
- Chinese-English synonym expansion
- Title-weighted BM25 (`bm25(fts, 10.0, 1.0)`)
- Content-hash deduplication via `topic_key`
- Multi-step retrieval guidance in MCP tool descriptions

## Benchmark Snapshot

### LoCoMo

Full [LoCoMo](https://github.com/snap-research/locomo) benchmark (10 conversations, 1540 QA pairs after adversarial skip):

| Config | Overall | Single-hop | Multi-hop | Temporal | Open-domain | Ingest | Model |
|---|---:|---:|---:|---:|---:|---|---|
| **v1 (fair)** | **56.8%** | 67.1% | 39.0% | 53.9% | 28.1% | per-turn | gpt-5.4 |
| **v2 (optimized)** | **62.7%** | 72.3% | 61.3% | 40.5% | 56.2% | session_summary | gpt-5.4 |

### Internal Eval (1777 real memories)

| Metric | Value |
|---|---:|
| MRR | 0.858 |
| Hit Rate@5 | 1.000 |
| Dedup rate | 1.0% |
| Project leak | 0% |
| Self-retrieval | 100% |

### Local QA Eval

```bash
python3 eval/local/run_local_eval.py --n 20
```

| Metric | Score |
|---|---:|
| Overall | **85.0%** |
| Decision | 77.8% |
| Discovery | 87.5% |
| Preference | 100% |
| Source in top-20 | 90.0% |

Requires `.env` with `OPENAI_API_KEY` (optional `OPENAI_BASE_URL`, `OPENAI_MODEL`).

## Token Usage And Cost Reporting

remem records an AI usage ledger for its own background extraction, summary,
compression, and promotion calls. The CLI can report daily and weekly token
usage and estimated cost:

```bash
remem usage --days 14 --weeks 8
remem usage --project /path/to/project --days 30 --weeks 12
```

The report includes calls, input tokens, cache tokens, output tokens, reasoning
tokens, total tokens, estimated USD cost, and a precision note. Usage rows are
tagged by source:

- `anthropic_usage`: provider-reported usage from the Anthropic Messages API
- `codex_log`: exact token counts parsed from the current `codex exec --json`
  `turn.completed.usage` event
- `text_estimate`: fallback estimate from prompt/response text length

Cost is an estimate, not an invoice. Historical rows may be text estimates or
may have been repriced from older rows that did not store the exact model.
Codex summarization defaults to `gpt-5.2`; set `REMEM_CODEX_MODEL=auto` to let
Codex choose its own default, or set any explicit Codex model name.

## Commands

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
remem cleanup
remem dream [--project X] [--dry-run]
remem install --target codex
remem mcp
remem sync-memory --cwd .
```

## REST API

```bash
remem api --port 5567
```

| Endpoint | Method | Description |
|---|---|---|
| `/api/v1/search?query=&project=&type=&limit=&offset=&branch=&multi_hop=` | GET | Search memories |
| `/api/v1/memory?id=` | GET | Get one memory |
| `/api/v1/memories` | POST | Save memory |
| `/api/v1/status` | GET | System status |

## Security

- SQLCipher encryption at rest (`remem encrypt`)
- Data directory permissions (`0700`)
- Key file permissions (`0600`)
- API binds localhost only (`127.0.0.1`)

## Architecture Docs

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for full internals and data flow.

## Uninstall

```bash
remem uninstall
rm -rf ~/.remem
```

## License

MIT
