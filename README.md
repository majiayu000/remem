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
- **Chinese↔English synonym expansion** (50+ term mappings)
- **Title-weighted BM25** (`bm25(fts, 10.0, 1.0)` — title matches 10x)
- **Hybrid routing** — long tokens → FTS5, short tokens → LIKE, merged with dedup

### Search Quality (eval on 953 real memories, 30 queries)

| Metric | Value |
|--------|-------|
| MRR | 0.272 |
| Recall@5 | 0.272 |
| Hit Rate@5 | 0.346 |

Run `remem eval` to benchmark on your own data.

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
| `/api/v1/search?query=&project=&limit=` | GET | Search memories |
| `/api/v1/memory?id=` | GET | Get single memory |
| `/api/v1/memories` | POST | Save a memory |
| `/api/v1/status` | GET | System status |

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
remem v0.2.0
  Memories:      1001
  Observations:  1834
  Entities:      1599
  Database:     138 MB
  Tests:         123 passing
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
- Database schema (v12)
- Design decisions

## Uninstall

```bash
remem uninstall    # Remove hooks and MCP config, data preserved
rm -rf ~/.remem    # Remove all data (optional)
```

## License

MIT
