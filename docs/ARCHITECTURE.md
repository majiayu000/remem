# Architecture

## System Overview

```
┌───────────────────────────────────────────────────────────┐
│              Host Hooks (Claude Code / Codex)              │
│                                                            │
│  Claude Code: SessionStart/UserPromptSubmit/PostToolUse/Stop│
│  Codex:       SessionStart/PostToolUse(Bash)/Stop           │
│                                                            │
│  SessionStart ──────→ context       (inject memories)      │
│  UserPromptSubmit ──→ session-init  (Claude Code only)     │
│  PostToolUse ───────→ observe       (Claude all, Codex Bash)│
│  Stop ──────────────→ summarize     (3-gate + worker)      │
└──────────────┬──────────────────────┬──────────────────────┘
               │                      │
               ▼                      ▼
┌──────────────────────┐  ┌──────────────────────────────────┐
│  MCP Server (stdio)  │  │  Background Worker (detached)     │
│                      │  │                                    │
│  search              │  │  1. flush (batch→obs, ≤15/batch)  │
│  get_observations    │  │  2. compress (>100→auto merge)     │
│  timeline            │  │  3. summarize (session summary)    │
│  timeline_report     │  │  4. promote (summary→memory)       │
│  save_memory         │  │                                    │
│  workstreams         │  │  Timeout: 180s global limit        │
│  update_workstream   │  │                                    │
└──────────┬───────────┘  └─────────────┬────────────────────┘
           │                            │
           ▼                            ▼
┌───────────────────────────────────────────────────────────┐
│              ~/.remem/remem.db (SQLite + WAL)              │
│                                                            │
│  pending_observations → observations → compressed          │
│  memories (decision/bugfix/preference/discovery/...)       │
│  session_summaries    workstreams    FTS5 full-text index   │
│  summarize_cooldown   ai_usage_events                      │
└───────────────────────────────────────────────────────────┘
```

## Module Overview (~9000 lines Rust)

| Module | Lines | Responsibility |
|--------|-------|----------------|
| `memory.rs` | 838 | Memory CRUD, auto-promotion from summaries, FTS search |
| `db.rs` | 728 | Data model + write ops + encryption + cleanup |
| `db_query.rs` | 680 | Read queries: FTS search, timeline, shared status stats |
| `observe_flush.rs` | 609 | Batch flush: pending→observations via AI |
| `workstream.rs` | 581 | WorkStream tracking across sessions (auto-create + fuzzy match) |
| `mcp/server.rs` | 565 | MCP service runtime: tools, server lifecycle, tests |
| `summarize.rs` | 501 | 3-gate + background worker + session summary + compression |
| `timeline.rs` | 493 | Timeline report with monthly aggregation |
| `cli/actions.rs` | 385 | CLI command implementations and formatted output |
| `context.rs` | 368 | Context rendering: preferences + core + index + workstreams + sessions |
| `preference.rs` | 352 | Preference management: query, render, CLI ops |
| `observe.rs` | 287 | Bash filter + event queuing + type checks |
| `db_pending.rs` | 261 | Pending observations management |
| `search.rs` | 251 | Search entry: filtered retrieval + pagination |
| `ai.rs` | 244 | AI calls: HTTP-first + CLI fallback + model mapping |
| `db_job.rs` | 244 | Background job queue |
| `install.rs` | 243 | Auto-configure hooks + MCP to settings.json |
| `claude_memory.rs` | 195 | Sync summaries to Claude Code native memory directory |
| `dedup.rs` | 179 | Hash-based deduplication |
| `cli/mod.rs` | 172 | CLI args + command dispatch |
| `log.rs` | 147 | Logging: file + stderr, Timer |
| `memory_format.rs` | 148 | XML memory format parsing |
| `db_models.rs` | 123 | Shared data models |
| `mcp/types.rs` | 121 | MCP parameter/result DTOs |
| `worker.rs` | 110 | Background worker loop |
| `vector.rs` | 80 | Vector similarity (SQLite vec extension) |
| `lib.rs` | 47 | Module declarations |
| `db_usage.rs` | 39 | AI usage statistics |
| `main.rs` | 6 | Binary entry: delegate to `cli::run()` |
| `mcp/mod.rs` | 4 | MCP public entry: export `run_mcp_server` |

## Data Flow

### 1. Observation Capture (PostToolUse → observe)

```
Tool call ──→ Type check ──→ Bash filter ──→ Queue to SQLite
               │              │
               │              └─ Skip: git status/log/diff, ls, cat,
               │                      npm install, cargo build (read-only)
               │
               └─ Accept: Claude Write/Edit/NotebookEdit/Bash/Task/Agent,
                          Codex Bash
                  Skip: Read, Glob, Grep, metadata-only tools
```

Each queued event stores: session_id, project, tool_name, tool_input, tool_response (truncated to 4KB).

### 2. Batch Distillation (Stop → summarize → flush)

```
Stop hook fires
       │
       ├─ Gate 1: pending < 3 → skip (filter short sessions)
       ├─ Gate 2: project cooldown 300s → skip (prevent duplicates)
       ├─ Gate 3: message hash match → skip (prevent duplicate content)
       │
       ▼ pass all gates
  spawn background worker (6ms return)
       │
       ├─ Worker re-checks Gate 2+3 (prevent parallel races)
       ├─ Pre-record cooldown (claim slot)
       │
       ▼
  flush_pending (≤15 events/batch)
       │
       ├─ Inject existing memories (delta dedup)
       ├─ Single AI call → structured observations
       ├─ File overlap detection → mark old observations stale
       │
       ▼
  summarize (session summary)
       │
       ├─ Inject same-session old summary (incremental merge)
       ├─ AI generates → replaces old summary
       │
       ▼
  promote (summary → memories)
       │
       ├─ Extract decisions, preferences, discoveries
       ├─ Upsert by topic_key (dedup across sessions)
       │
       ▼
  maybe_compress (long-term compression)
       │
       └─ >100 active observations → oldest 30 merged into 1-2 summaries
```

### 3. Context Injection (SessionStart → context)

```
New session starts
       │
       ▼
  Load preferences (project + global)
       │
       ├─ Project preferences from memories table
       ├─ Global preferences (topic_key in 3+ projects)
       ├─ Dedup against CLAUDE.md (skip already present)
       │
       ▼
  Load recent 50 memories + 5 session summaries
       │
       ├─ Branch-aware: current branch first, then main, then others
       ├─ Score-based: decision > bugfix > architecture > discovery
       ├─ Core section: top 6 scored, 200-char preview
       ├─ Index section: grouped by type
       │
       ▼
  Render to stdout → Claude Code injects into CLAUDE.md
       │
       ├─ "Your Preferences" section (always apply)
       ├─ Core memories with preview
       ├─ Memory index by type
       ├─ Active workstreams with status + next action
       └─ Recent session summaries (request/completed)
```

### 4. Stale Queue Recovery (Claude Code UserPromptSubmit → session_init)

```
New message submitted
       │
       ├─ Register/update session
       │
       ▼
  Scan same-project pending older than 10 minutes
       │
       └─ Auto flush → prevent low-activity session observation loss
```

## Memory Lifecycle

```
Tool operations ──→ pending_observations (raw queue, ≤4KB/event)
                         │
                         ▼ flush (≤15 events/batch, single AI call)
                  observations (structured memory)
                         │
             ┌───────────┼───────────┐
             ▼           ▼           ▼
          active      stale      compressed
        (normal      (file        (>100 active
         display)   overlap,      → auto merge)
                   lower rank)
                                     │
                                     ▼ 90 days
                                  deleted (cleanup command)
```

```
session_summaries ──→ memories (auto-promoted)
                         │
                    decision / bugfix / preference / discovery / architecture
                         │
                         ▼ used in context injection
                    "Your Preferences" section + Core + Index
```

- **Incremental delta**: During flush, inject latest 10 existing memories so AI skips duplicates
- **File overlap staleness**: When new operations overwrite old files, old observations auto-marked stale
- **Time decay**: FTS search ranked by relevance × time decay, stale observations further penalized
- **Auto compression**: Projects with >100 observations: keep newest 50, merge oldest 30 into 1-2 summaries
- **TTL cleanup**: Compressed records older than 90 days deleted by `cleanup` command

## Rate Limiting

Short-lived process model (each hook = independent process) cannot dedup via in-memory state. remem uses SQLite to implement 3-gate rate limiting:

| Gate | Mechanism | Intercepts |
|------|-----------|------------|
| Gate 1 | `pending < 3` skip | Short sessions (1-2 operations then exit) |
| Gate 2 | Project cooldown 300s | Same-project rapid summarize |
| Gate 3 | Message hash dedup | Identical assistant messages |
| Worker double-check | Re-verify Gate 2+3 on entry | Parallel worker races |
| Pre-claim | Record cooldown before AI call | Prevent parallel workers passing gate simultaneously |

`summarize_cooldown` table stores each project's last summarize time and message hash.

## AI Calls

```
              ┌─ ANTHROPIC_API_KEY exists?
              │
         Yes ─┤──→ HTTP API direct (2-5s)
              │         │ fails
              │         ▼
         No ──┴──→ claude -p CLI (30-60s)
```

- **Model mapping**: `REMEM_MODEL=haiku` → `claude-haiku-4-5-20251001` (HTTP uses full ID, CLI uses short name)
- **Timeouts**: Single AI call 90s, entire worker 180s
- **4 prompts**: observation (capture), summary (session summary), compress (long-term compression), promote (summary→memory)

## MCP Server

MCP server via stdio transport, providing 7 tools:

| Tool | Description |
|------|-------------|
| `search` | Full-text search (FTS5) + project/type filter, returns ID+title |
| `get_observations` | Get full memory by ID (narrative, facts, concepts, files) |
| `timeline` | Timeline query: observations around an anchor point |
| `timeline_report` | Project history and Token ROI report |
| `save_memory` | Manually save important memories with local Markdown backup |
| `workstreams` | List active high-level tasks tracked across sessions |
| `update_workstream` | Update workstream status, next action, or blockers |

Recommended workflow: `search(query)` → find relevant IDs → `get_observations(ids)` for full content.

`save_memory` behavior:
- Dual-write by default: SQLite memory + local Markdown (`~/.remem/manual-notes/<project>/...md`)
- Custom local path via `local_path` parameter
- When user asks to "save a document", write project-local file first, then `save_memory` as long-term backup

## Memory Scope (Project vs Global)

Memories have a `scope` field: `project` (default) or `global`.

| Scope | Visibility | Auto-assigned to |
|-------|-----------|------------------|
| `project` | Only in the originating project | decision, bugfix, discovery, architecture |
| `global` | All projects | preference |

**How it works automatically:**
- When a session summary is promoted to memories, `preference` type automatically gets `scope=global`
- When Claude calls `save_memory(type="preference")`, scope defaults to `global`
- Other types (decision, bugfix, etc.) stay `project`-scoped
- Context injection query: `WHERE (project = ? OR scope = 'global')`

**No manual action needed.** Preferences learned in project A automatically appear in project B's context.

The `save_memory` MCP tool accepts an optional `scope` parameter for explicit control. The CLI supports `remem preferences add --global "text"` for manual global preferences.

## Project Identification

Project key = `last two path segments + canonical absolute path hash`, balancing readability and uniqueness:

```
/Users/foo/code/my-app       → code/my-app@9c1e2f3a4b5c
/Users/foo/personal/my-app   → personal/my-app@7a8b9c0d1e2f
/Users/foo/Desktop/code/AI/tools/remem → tools/remem@b7f8a1d44c2e
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `REMEM_DATA_DIR` | `~/.remem` | Data directory (DB + logs) |
| `REMEM_MODEL` | `haiku` | AI model (haiku/sonnet/opus or full model ID) |
| `REMEM_EXECUTOR` | `auto` | Legacy/general AI executor fallback for summaries and unspecified operations: `auto` / `http` / `claude-cli` / `codex-cli` |
| `REMEM_SUMMARY_EXECUTOR` | `REMEM_EXECUTOR` | Summary executor override, used by Stop hooks (`claude-cli` for Claude Code, `codex-cli` for Codex) |
| `REMEM_FLUSH_EXECUTOR` | `auto` | Flush/background observation executor override. If unset, `flush` / `flush-task` reuse `REMEM_SUMMARY_EXECUTOR` only when it resolves to Codex, so older Codex installs keep working without broadening Claude behavior |
| `REMEM_COMPRESS_EXECUTOR` | `auto` | Memory compression executor override |
| `REMEM_DREAM_EXECUTOR` | `auto` | Dream executor override |
| `ANTHROPIC_API_KEY` | - | Required for HTTP mode (also supports `ANTHROPIC_AUTH_TOKEN`) |
| `ANTHROPIC_BASE_URL` | `https://api.anthropic.com` | Custom API endpoint |
| `REMEM_DEBUG` | - | Enable debug logging |
| `REMEM_CONTEXT_HOST` | `auto` | Context host profile override: `claude-code`, `codex-cli`, or `unknown` |
| `REMEM_CONTEXT_TOTAL_CHAR_LIMIT` | `12000` | Soft character cap for rendered context |
| `REMEM_CONTEXT_CANDIDATE_FETCH_LIMIT` | `120` | Candidate memories fetched before section selection |
| `REMEM_CONTEXT_MEMORY_INDEX_LIMIT` | `50` | Non-preference memories shown in the main memory index |
| `REMEM_CONTEXT_OBSERVATIONS` | `50` | Deprecated alias for `REMEM_CONTEXT_MEMORY_INDEX_LIMIT` |
| `REMEM_CONTEXT_MEMORY_INDEX_CHAR_LIMIT` | `4000` | Main memory index character budget |
| `REMEM_CONTEXT_CORE_ITEM_LIMIT` | `6` | Core memory item budget |
| `REMEM_CONTEXT_CORE_CHAR_LIMIT` | `3000` | Core memory character budget |
| `REMEM_CONTEXT_SESSION_COUNT` | `5` | Session summaries shown |
| `REMEM_CONTEXT_SELF_DIAGNOSTIC_LIMIT` | `2` | Self-diagnostic memory cap |
| `REMEM_CONTEXT_PREFERENCE_PROJECT_LIMIT` | `20` | Project preference query limit |
| `REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT` | `10` | Global preference query limit |
| `REMEM_CONTEXT_PREFERENCE_CHAR_LIMIT` | `1500` | Preference section character budget |
| `REMEM_CLAUDE_PATH` | `claude` | Claude CLI path |
| `REMEM_CODEX_PATH` | `codex` | Codex CLI path |
| `REMEM_CODEX_MODEL` | - | Optional Codex CLI model override |
| `REMEM_LOG_MAX_BYTES` | `10485760` | Log file size limit (bytes), auto-rotated |
| `REMEM_SAVE_MEMORY_LOCAL_COPY` | `true` | Enable local Markdown backup for save_memory |
| `REMEM_SAVE_MEMORY_LOCAL_DIR` | `~/.remem/manual-notes` | Local backup directory |
| `REMEM_PRICE_INPUT_PER_MTOK` | model default | Override all models input price (USD/M tokens) |
| `REMEM_PRICE_OUTPUT_PER_MTOK` | model default | Override all models output price (USD/M tokens) |
| `REMEM_PRICE_HAIKU_INPUT_PER_MTOK` | `0.8` | Haiku input price |
| `REMEM_PRICE_HAIKU_OUTPUT_PER_MTOK` | `4.0` | Haiku output price |
| `REMEM_PRICE_SONNET_INPUT_PER_MTOK` | `3.0` | Sonnet input price |
| `REMEM_PRICE_SONNET_OUTPUT_PER_MTOK` | `15.0` | Sonnet output price |
| `REMEM_PRICE_OPUS_INPUT_PER_MTOK` | `15.0` | Opus input price |
| `REMEM_PRICE_OPUS_OUTPUT_PER_MTOK` | `75.0` | Opus output price |

## Data Cleanup

```bash
remem cleanup    # One-command cleanup
```

Cleans:
- Orphan summaries (`mem-*` prefix with no corresponding observation)
- Duplicate summaries (same session+project, keep newest)
- Expired pending (>1 hour unprocessed)
- Expired compressed (>90 days)

## Database Schema

```sql
-- Tool event queue
pending_observations (session_id, project, tool_name, tool_input, tool_response, cwd,
                      created_at_epoch, lease_owner, lease_expires_epoch)

-- Structured observations (AI-distilled from tool events)
observations (memory_session_id, project, type, title, subtitle, narrative, facts, concepts,
              files_read, files_modified, status[active|stale|compressed], discovery_tokens)

-- Long-term memories (auto-promoted from summaries + manual save)
memories (session_id, project, topic_key, title, content, memory_type, files, branch,
          created_at_epoch, updated_at_epoch, status, scope[project|global])

-- Session summaries
session_summaries (memory_session_id, project, request, completed, decisions, learned,
                   next_steps, preferences, discovery_tokens)

-- WorkStreams (cross-session task tracking)
workstreams (project, title, status, next_action, blockers,
             created_at_epoch, updated_at_epoch)

-- Session mapping
sdk_sessions (content_session_id → memory_session_id, project, prompt_counter)

-- Rate limiting
summarize_cooldown (project, last_summarize_epoch, last_message_hash)

-- AI call statistics
ai_usage_events (created_at_epoch, project, operation, executor, model,
                 input_tokens, output_tokens, total_tokens, estimated_cost_usd)

-- Full-text indexes
observations_fts (title, subtitle, narrative, facts, concepts)  -- FTS5 trigram
memories_fts (title, content)                                    -- FTS5 trigram
```

## Design Decisions

- **Short-lived process model**: Each hook call = independent process, zero shared state, <6ms response, never blocks Claude Code
- **SQLite constraint compensation**: No in-memory Map dedup capability, DB tables (`summarize_cooldown`) simulate rate limiting
- **HTTP-first AI calls**: HTTP API direct 2-5s vs `claude -p` CLI 30+s, 6-12x performance gap
- **Stop hook async**: Dispatcher returns in 6ms, `std::process::Command` spawns independent worker
- **SQLite single-file + WAL**: Zero dependencies, FTS5 full-text search, WAL concurrent read/write
- **Queue batch processing**: Claude Code PostToolUse only queues (<1ms), Stop processes ≤15 events in one AI call
- **Decision priority**: Summary fields ordered decisions > completed > learned, architectural knowledge most valuable
- **Schema version control**: `PRAGMA user_version` skips repeated migration, reduces per-hook DB overhead
- **Stable project key**: `parent/dirname@hash12`, readable prefix + canonical path hash, eliminates same-name directory collisions
- **Branch-aware memories**: Memories tagged with git branch, current branch prioritized in context
- **Auto-promotion**: Session summaries automatically distilled into typed memories (decision/bugfix/preference/discovery)
- **Preference-first context**: Preferences rendered before core memories, always visible at session start
- **Global scope for preferences**: Preferences auto-scoped as `global`, visible across all projects without manual action. Other memory types stay `project`-scoped. Inspired by Augment's User Rules vs Workspace Rules separation
