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

## Commands

```bash
remem install              # Configure hooks + MCP server
remem uninstall            # Remove hooks + MCP (data preserved)
remem status               # Show system health and statistics
remem preferences list     # View all preferences
remem preferences add "text"  # Add a preference manually
remem preferences remove 42   # Remove a preference by ID
remem context --cwd .      # Preview context injection (debug)
remem cleanup              # Clean old events and stale memories
remem mcp                  # Start MCP server (used by Claude Code)
remem sync-memory --cwd .  # Sync summaries to Claude Code native memory
```

## remem vs Built-in Memory

| Feature | Claude Code Memory | remem |
|---|---|---|
| Capture method | Manual (`save_memory`) | Automatic (hooks) |
| Cross-session context | ~5 recent memories | 50+ scored memories |
| Preferences | Mixed with other content | Dedicated section, always visible |
| Decision tracking | Not specialized | Type-aware (decision/bugfix/discovery) |
| Search | Basic | FTS5 full-text with CJK support |
| Branch awareness | No | Branch-scoped memories |
| Session summaries | No | Auto-generated with request/completed/decisions |
| WorkStream tracking | No | Cross-session task tracking with status |

## Real-world Usage

After 1 month of production use:

```
remem v0.2.0
  Memories:       656
  Observations:  1670
  Sessions:      1076
  Database:     120 MB
```

## Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for detailed documentation including:

- System architecture diagram
- Module overview (~8200 lines across 26 modules)
- Data flow (observation capture → distillation → context injection)
- Memory lifecycle (pending → observations → memories)
- Rate limiting (3-gate system)
- AI call strategy (HTTP-first + CLI fallback)
- MCP Server (7 tools)
- Environment variables (full list)
- Database schema
- Design decisions

## Uninstall

```bash
remem uninstall    # Remove hooks and MCP config, data preserved
rm -rf ~/.remem    # Remove all data (optional)
```

## License

MIT
