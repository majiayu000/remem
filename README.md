# remem

> Stop re-explaining your project every new session.

Persistent memory for Claude Code. A single Rust binary that automatically captures, distills, and injects your project context across sessions — decisions, patterns, preferences, and learnings.

[![CI](https://github.com/majiayu000/remem/actions/workflows/ci.yml/badge.svg)](https://github.com/majiayu000/remem/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## The Problem

- **Session amnesia**: Every new Claude Code session starts from zero — you re-explain architecture, conventions, and past decisions
- **Lost context**: Bug fixes, design rationale, and "why we did X" vanish when the session ends
- **Preference fatigue**: You correct the same behaviors ("use Chinese comments", "don't mock the database") session after session
- **No continuity**: Working on a feature across multiple sessions means constantly rebuilding context

## How remem Solves This

| Without remem | With remem |
|---|---|
| "We use FTS5 with trigram tokenizer for CJK..." (every session) | Automatically injected from memory |
| "Don't use `expect()` in non-test code" (again) | Preference applied before you ask |
| "Last session we decided to..." (reconstructing from git log) | Full decision history with rationale |
| Bug fix context lost after session ends | Root cause + fix preserved as memory |

## Install

```bash
# Option 1: Quick install (downloads pre-built binary)
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | sh

# Option 2: Cargo
cargo install remem

# Option 3: Build from source
git clone https://github.com/majiayu000/remem.git
cd remem
cargo build --release
cp target/release/remem ~/.local/bin/

# Then configure Claude Code hooks + MCP:
remem install
```

Restart Claude Code after installation. remem starts working automatically.

## How It Works

remem runs silently through Claude Code's Hooks system:

```
Your normal Claude Code workflow
        │
        ├─ SessionStart      → Injects memories + preferences into context
        ├─ UserPromptSubmit  → Registers session, flushes stale queues
        ├─ PostToolUse       → Captures tool operations (queued, <1ms)
        └─ Stop              → Summarizes session in background (6ms return)
```

**You don't need to do anything** — capture, distillation, and retrieval are fully automatic.

Memories are scoped by project. **Preferences** (coding style, tool choices) are automatically shared across all projects — learn once, apply everywhere.

## Search Architecture

remem uses a **4-channel Reciprocal Rank Fusion (RRF)** search inspired by [Hindsight](https://github.com/vectorize-io/hindsight):

```
Query: "数据库加密"
        │
   ┌────┴────────────────────────────────┐
   │          4 parallel channels         │
   ├─────────────────────────────────────┤
   │ 1. FTS5 (BM25)    trigram + OR      │
   │ 2. Entity Index    1600+ entities    │
   │ 3. Temporal        "昨天"/"last week"│
   │ 4. LIKE fallback   short tokens      │
   └──────────┬──────────────────────────┘
              │
        RRF Fusion: score = Σ 1/(60 + rank_i)
              │
        Top-K results sorted by fused score
```

Additional search enhancements:
- **CJK dictionary segmentation** — "数据库加密" → "数据库" + "加密" → database + encrypt
- **Chinese↔English synonym expansion** (90+ term mappings)
- **Title-weighted BM25** (`bm25(fts, 10.0, 1.0)` — title matches 10x)
- **Hybrid routing** — long tokens → FTS5, short tokens → LIKE, merged with dedup
- **Core-token LIKE** — LIKE channel uses CJK-segmented original tokens (no synonym noise)

### LoCoMo Benchmark

Evaluated on the full [LoCoMo](https://github.com/snap-research/locomo) benchmark — 10 conversations, 1540 QA pairs (skipping adversarial category, same as Mem0). All results and raw outputs are in [`eval/locomo/results/`](eval/locomo/results/).

**remem results (two configurations):**

| Config | Overall | Single-hop | Multi-hop | Temporal | Open-domain | Ingest | Gen/Judge Model |
|--------|---------|------------|-----------|----------|-------------|--------|-----------------|
| **v1** (fair) | **56.8%** | 67.1% | 39.0% | 53.9% | 28.1% | per-turn | gpt-5.4 |
| **v2** (optimized) | **62.7%** | 72.3% | 61.3% | 40.5% | 56.2% | session_summary | gpt-5.4 |

**Competitor comparison:**

| System | Overall | Gen Model | Judge Model | Ingest Strategy | Source |
|--------|---------|-----------|-------------|-----------------|--------|
| Hindsight | 89.6% | Gemini-3 | GPT-4o-mini | LLM fact extraction | [paper](https://arxiv.org/abs/2512.12818) |
| Letta filesystem | 74.0% | GPT-4o-mini | GPT-4o-mini | per-session files | [blog](https://www.letta.com/blog/benchmarking-ai-agent-memory) |
| Mem0 (self-reported) | 68.5% | GPT-4o | GPT-4o-mini | LLM memory extraction | [paper](https://arxiv.org/abs/2504.19413) |
| Mem0 (third-party) | ~58% | — | — | — | [issue](https://github.com/getzep/zep-papers/issues/5) |
| **remem v1** | **56.8%** | gpt-5.4 | gpt-5.4 | per-turn raw | this repo |
| RAG baseline | ~55% | GPT-4o | GPT-4o-mini | chunk+embed | Mem0 paper |
| Full-context | ~39% | GPT-4o | GPT-4o-mini | all in context | LoCoMo paper |

> **Fairness notes:**
> - **v1 (56.8%)** is the fair comparison — per-turn ingest is closest to Mem0's method. remem uses gpt-5.4 (stronger than Mem0's gpt-4o), which may inflate scores by ~2-5pp.
> - **v2 (62.7%)** uses LoCoMo's pre-built `session_summary` (human-annotated), which other systems don't use. This shows search ceiling with ideal ingest quality, not real-world performance.
> - All systems use different LLM models for generation and judging, making exact comparison imprecise. Run the benchmark yourself for apples-to-apples: `python eval/locomo/run_locomo.py`
> - remem uses no vector search — pure FTS5 + SQLite + RRF fusion.

### Internal Search Quality (eval on 1001 real memories, 30 queries)

| Metric | Value |
|--------|-------|
| MRR | 0.858 |
| Precision@5 | 0.460 |
| Recall@5 | 0.628 |
| Hit Rate@5 | 1.000 |

Measured with `remem eval` against a [calibrated golden dataset](eval/golden.json).

## Commands

```bash
remem install              # Configure hooks + MCP server
remem uninstall            # Remove hooks + MCP (data preserved)
remem doctor               # System health check (6 checks)
remem search "query"       # Search memories from CLI
remem show <id>            # Show memory details
remem eval                 # Run search quality benchmark
remem backfill-entities    # Populate entity index from existing memories
remem encrypt              # Encrypt database with SQLCipher
remem api --port 5567      # Start REST API server
remem status               # Show system health and statistics
remem pending list-failed  # Show failed pending observations
remem pending retry-failed # Requeue failed pending observations
remem pending purge-failed # Purge old failed pending observations
remem preferences list     # View all preferences
remem preferences add "text"  # Add a preference manually
remem preferences remove 42   # Remove a preference by ID
remem context --cwd .      # Preview context injection (debug)
remem cleanup              # Clean old events and stale memories
remem mcp                  # Start MCP server (used by Claude Code)
remem sync-memory --cwd .  # Sync summaries to Claude Code native memory
```

## REST API

remem includes an Axum-based REST API for cross-platform integration:

```bash
remem api --port 5567
```

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/v1/search?query=&project=&type=&limit=&offset=&branch=&multi_hop=` | GET | Search memories |
| `/api/v1/memory?id=` | GET | Get single memory |
| `/api/v1/memories` | POST | Save a memory |
| `/api/v1/status` | GET | System status |

### `POST /api/v1/memories` body

```json
{
  "text": "Switched tokenizer to FTS5 trigram for CJK retrieval.",
  "title": "FTS5 trigram decision",
  "project": "/Users/lifcc/Desktop/code/AI/tools/remem",
  "topic_key": "fts5-trigram-search",
  "memory_type": "decision",
  "files": ["src/search.rs", "src/db_query.rs"],
  "scope": "project",
  "created_at_epoch": 1774500000,
  "branch": "main",
  "local_path": "docs/notes/fts5.md",
  "local_copy_enabled": true
}
```

### `GET /api/v1/search` response shape

```json
{
  "data": [
    {
      "id": 101,
      "title": "FTS5 trigram decision",
      "content": "Switched tokenizer...",
      "memory_type": "decision",
      "project": "/Users/lifcc/Desktop/code/AI/tools/remem",
      "scope": "project",
      "status": "active",
      "topic_key": "fts5-trigram-search",
      "branch": "main",
      "created_at_epoch": 1774500000,
      "updated_at_epoch": 1774500000
    }
  ],
  "meta": {
    "count": 1,
    "has_more": false,
    "limit": 20,
    "offset": 0
  },
  "multi_hop": {
    "hops": 2,
    "entities_discovered": ["FTS5", "SQLite"]
  }
}
```

## Security

- **SQLCipher encryption**: `remem encrypt` encrypts the database at rest
- **File permissions**: Data directory `0700`, log files `0600`
- **Key storage**: `~/.remem/.key` with `0600` permissions
- **Encryption key**: Set `REMEM_CIPHER_KEY` env var or use auto-generated key file
- **API binding**: REST API binds `127.0.0.1` only (localhost)

## Multi-Tool Support

remem's `ToolAdapter` trait enables support for multiple AI coding tools:

```rust
pub trait ToolAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn parse_hook(&self, raw_json: &str) -> Option<ParsedHookEvent>;
    fn should_skip(&self, event: &ParsedHookEvent) -> bool;
    fn classify_event(&self, event: &ParsedHookEvent) -> Option<EventSummary>;
}
```

Currently supports Claude Code. Future: Codex, Cursor, Aider — implement the trait only.

## remem vs Built-in Memory

| Feature | Claude Code Memory | remem |
|---|---|---|
| Capture method | Manual (`save_memory`) | Automatic (hooks) |
| Cross-session context | ~5 recent memories | 50+ scored memories |
| Preferences | Mixed with other content | Dedicated section, always visible |
| Decision tracking | Not specialized | Type-aware (decision/bugfix/discovery) |
| Search | Basic | 4-channel RRF fusion with entity index |
| Branch awareness | No | Branch-scoped memories |
| Cross-project sharing | No | Preferences auto-shared globally |
| Session summaries | No | Auto-generated with request/completed/decisions |
| WorkStream tracking | No | Cross-session task tracking with status |
| Database encryption | No | SQLCipher at rest |
| CLI tools | No | `doctor`, `search`, `show`, `eval` |
| REST API | No | Axum HTTP server |

## Real-world Usage

After 1 month of production use:

```
remem v0.3.5
  Memories:      1001
  Observations:  1834
  Entities:      1599
  Database:     138 MB
  Tests:         128 passing
  Search MRR:    0.858
  Hit Rate@5:    1.000
```

## Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for detailed documentation including:

- System architecture diagram
- Module overview (~10,200 lines across 30 modules)
- Data flow (observation capture → distillation → context injection)
- Memory lifecycle (pending → observations → memories)
- 4-channel RRF search (FTS5 + Entity + Temporal + LIKE)
- Rate limiting (3-gate system)
- AI call strategy (HTTP-first + CLI fallback)
- MCP Server (7 tools)
- REST API (Axum)
- Environment variables (full list)
- Database schema (v13)
- Design decisions

## Uninstall

```bash
remem uninstall    # Remove hooks and MCP config, data preserved
rm -rf ~/.remem    # Remove all data (optional)
```

## License

MIT
