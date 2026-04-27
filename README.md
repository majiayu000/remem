# remem

> Stop re-explaining your project every new session.

Language: **English** | [简体中文](README.zh-CN.md)

Persistent memory for Claude Code and Codex. A single Rust binary that automatically captures, distills, and injects project context across sessions: decisions, patterns, preferences, and learnings.

[![CI](https://github.com/majiayu000/remem/actions/workflows/ci.yml/badge.svg)](https://github.com/majiayu000/remem/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## The Problem

- **Session amnesia**: every new Claude Code session starts from zero.
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
# Option 1: Quick install (prebuilt binary)
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | sh

# Option 2: Cargo
cargo install remem-ai --bin remem

# Option 3: Build from source
git clone https://github.com/majiayu000/remem.git
cd remem
cargo build --release
cp target/release/remem ~/.local/bin/
codesign -s - -f ~/.local/bin/remem  # required on macOS ARM

# Configure detected Claude Code/Codex hooks + MCP
remem install
```

Restart your AI coding tool after installation.

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

No manual capture is required.

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

## Commands

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
remem dream [--project X] [--dry-run]
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
