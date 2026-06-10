# Architecture

## System Overview

```
┌───────────────────────────────────────────────────────────┐
│              Host Hooks (Claude Code / Codex)              │
│                                                            │
│  Claude Code: SessionStart/UserPromptSubmit/PostToolUse/Stop│
│  Codex:       SessionStart/Stop                             │
│                                                            │
│  SessionStart ──────→ context       (inject memories)      │
│  UserPromptSubmit ──→ session-init  (Claude Code only)     │
│  PostToolUse ───────→ observe       (Claude Code)           │
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
│  timeline_report     │  │  4. candidate (summary→review)      │
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

Codex legacy `PostToolUse(Bash)` observe hooks are treated as opt-in only:
they are skipped unless `REMEM_ENABLE_CODEX_BASH_OBSERVE=1` is set. This keeps
Bash-heavy sessions from creating an unbounded pending-observation backlog
before the coalesced capture path is enabled.

The capture path is now present in the main database as a first production
slice: hooks write append-only `captured_events`, large evidence goes to
`event_blobs`, and `extraction_tasks` coalesces pending extraction by
host/project/session/task kind. This ledger is evidence and scheduling state;
durable memory is still created only after extraction, candidate review, and
promotion.

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

### 1. Capture Ledger (hook/session evidence → captured_events)

```
Hook/session payload
       │
       ├─ Normalize host/workspace/project/session identity
       ├─ Store raw evidence in captured_events/event_blobs
       └─ Coalesce extraction_tasks by host/project/session/task kind
```

This path is intentionally light: it does not call an LLM and it does not
create one job per tool call.

### 2. Legacy Observation Capture (Claude PostToolUse → observe)

```
Tool call ──→ Type check ──→ Bash filter ──→ Queue to SQLite
               │              │
               │              └─ Skip: git status/log/diff, ls, cat,
               │                      npm install, cargo build (read-only)
               │
               └─ Accept: Claude Write/Edit/NotebookEdit/Bash/Task/Agent
                  Skip: Read, Glob, Grep, metadata-only tools
```

Each queued event stores: session_id, project, tool_name, tool_input, tool_response (truncated to 4KB).

### 3. Batch Distillation (Stop → summarize → flush)

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

The legacy Summary job remains the canonical source for SessionStart recent
session context because it also drives summary-derived candidates, workstream
updates, raw archive ingest, and native-memory sync. Capture-ledger
`SessionRollup` rows are event-range artifacts keyed by `session_row_id` and
coverage columns; they may coexist in `session_summaries`, but recent-session
context queries exclude rows with `session_row_id IS NOT NULL` so the two
pipelines cannot surface duplicate user-facing session summaries.

### 3. Context Injection (SessionStart → context)

```
New session starts
       │
       ▼
  Load preferences (project + explicit global opt-in)
       │
       ├─ Project preferences from memories table
       ├─ Global preferences only when the global limit is explicitly enabled
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
          active      stale      compressed source
        (normal      (file        (>100 active
         display)   overlap,      → auto merge)
                   lower rank)
                                     │
                                     ▼ 90 days after compression link
                                  deleted only with source hash/snapshot provenance
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
- **Retention cleanup**: Compression replacement observations are retained; retired
  source observations can be deleted 90 days after compression only when
  `compressed_observation_sources` still has sufficient hash/snapshot provenance

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
UsageContext { host/profile }
        │
        ├─ profile set? ───────────→ [memory_ai.profiles.<profile>]
        │
        └─ host/default_host ──────→ [memory_ai.hosts."<host>"].memory_profile
                                      │
                                      ▼
                             [memory_ai.profiles.<name>]
                                      │
           ┌──────────────┬──────────┴──────────┬──────────────┐
           ▼              ▼                     ▼              ▼
      executor=http  executor=claude-cli  executor=codex-cli  usage ledger
```

- **Config path**: `~/.remem/config.toml`, override with `REMEM_CONFIG`
- **Default Codex profile**: executor `codex-cli`, model `gpt-5.2`
- **Model mapping**: profile model `haiku` maps to `claude-haiku-4-5-20251001` for Anthropic HTTP; CLI executors receive the configured model string directly
- **Codex model `auto`**: omit `--model` and use the Codex CLI default
- **Timeouts**: Single AI call 90s, entire worker 180s
- **Unified prompts**: summarize, session rollup, observation extract, memory candidate, compress, and dream all resolve through the same host/profile config
- **Usage ledger**: `ai_usage_events` stores model, operation, token breakdown, usage source, pricing source, and estimated USD cost
- **Precision levels**: provider/log usage (`anthropic_usage`, `codex_log`) is preferred; `text_estimate` is kept only as a fallback and marked in reports

Default generated config:

```toml
version = 1

[memory_ai]
default_host = "codex-cli"

[memory_ai.hosts."codex-cli"]
memory_profile = "codex"
context_gate = "strict"
context_color = true
capture_adapter = "codex-cli"

[memory_ai.hosts."claude-code"]
memory_profile = "claude"
context_gate = "off"
context_color = true
capture_adapter = "claude-code"

[memory_ai.profiles.codex]
executor = "codex-cli"
model = "gpt-5.2"
path = "codex"

[memory_ai.profiles.claude]
executor = "claude-cli"
model = "haiku"
path = "claude"

[memory_ai.profiles.anthropic_http]
executor = "http"
model = "haiku"
base_url = "https://api.anthropic.com"
```

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
| `project` | Only in the originating project | decision, bugfix, discovery, architecture, preference |
| `global` | All projects | Explicit opt-in only |

**How it works automatically:**
- Summary-derived durable facts become `memory_candidates`; they do not directly write active `memories`.
- New active memory writes populate `source_project`, `target_project`, `owner_scope`, and `owner_key`.
- SessionStart context uses owner-aware startup filters: repo-owned rows for the current repo, user-owned preferences, and legacy project rows only as a compatibility fallback.
- Tool/domain-owned memories are excluded from startup context unless later task-aware retrieval explicitly asks for that owner class.
- The context footer reports owner counts (`repo`, `user`, `tool`, `domain`, etc.), and `--debug` shows inclusion/exclusion reasons.

User preferences require explicit user/global ownership. Project preferences learned in project A do not automatically appear in project B's repo context.

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
| `REMEM_CONFIG` | `~/.remem/config.toml` | Runtime config file for memory-AI host/profile policy |
| `ANTHROPIC_API_KEY` | - | Required for HTTP mode (also supports `ANTHROPIC_AUTH_TOKEN`) |
| `REMEM_DEBUG` | - | Enable debug logging |
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
| `REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT` | `0` | Global preference query limit; disabled by default |
| `REMEM_CONTEXT_PREFERENCE_CHAR_LIMIT` | `1500` | Preference section character budget |
| `REMEM_LOG_MAX_BYTES` | `10485760` | Log file size limit (bytes), auto-rotated |
| `REMEM_SAVE_MEMORY_LOCAL_COPY` | `true` | Enable local Markdown backup for save_memory |
| `REMEM_SAVE_MEMORY_LOCAL_DIR` | `~/.remem/manual-notes` | Local backup directory |
| `REMEM_PRICE_INPUT_PER_MTOK` | model default | Override all models input price (USD/M tokens) |
| `REMEM_PRICE_OUTPUT_PER_MTOK` | model default | Override all models output price (USD/M tokens) |
| `REMEM_PRICE_REASONING_PER_MTOK` | output price | Override all models reasoning token price |
| `REMEM_PRICE_CACHE_CREATION_PER_MTOK` | input price | Override all models cache creation price |
| `REMEM_PRICE_CACHE_READ_PER_MTOK` | input price | Override all models cache read price |
| `REMEM_PRICE_HAIKU_INPUT_PER_MTOK` | `1.0` | Haiku input price |
| `REMEM_PRICE_HAIKU_OUTPUT_PER_MTOK` | `5.0` | Haiku output price |
| `REMEM_PRICE_HAIKU_REASONING_PER_MTOK` | output price | Haiku reasoning token price |
| `REMEM_PRICE_HAIKU_CACHE_CREATION_PER_MTOK` | `1.25` | Haiku cache creation price |
| `REMEM_PRICE_HAIKU_CACHE_READ_PER_MTOK` | `0.10` | Haiku cache read price |
| `REMEM_PRICE_SONNET_INPUT_PER_MTOK` | `3.0` | Sonnet input price |
| `REMEM_PRICE_SONNET_OUTPUT_PER_MTOK` | `15.0` | Sonnet output price |
| `REMEM_PRICE_SONNET_REASONING_PER_MTOK` | output price | Sonnet reasoning token price |
| `REMEM_PRICE_SONNET_CACHE_CREATION_PER_MTOK` | `3.75` | Sonnet cache creation price |
| `REMEM_PRICE_SONNET_CACHE_READ_PER_MTOK` | `0.30` | Sonnet cache read price |
| `REMEM_PRICE_OPUS_INPUT_PER_MTOK` | `15.0` | Opus input price |
| `REMEM_PRICE_OPUS_OUTPUT_PER_MTOK` | `75.0` | Opus output price |
| `REMEM_PRICE_OPUS_REASONING_PER_MTOK` | output price | Opus reasoning token price |
| `REMEM_PRICE_OPUS_CACHE_CREATION_PER_MTOK` | `18.75` | Opus cache creation price |
| `REMEM_PRICE_OPUS_CACHE_READ_PER_MTOK` | `1.50` | Opus cache read price |
| `REMEM_PRICE_GPT5_CODEX_INPUT_PER_MTOK` | `1.75` | GPT-5.2 / GPT-5.3-Codex input price |
| `REMEM_PRICE_GPT5_CODEX_OUTPUT_PER_MTOK` | `14.0` | GPT-5.2 / GPT-5.3-Codex output price |
| `REMEM_PRICE_GPT5_CODEX_REASONING_PER_MTOK` | output price | GPT-5.2 / GPT-5.3-Codex reasoning token price |
| `REMEM_PRICE_GPT5_CODEX_CACHE_CREATION_PER_MTOK` | `0.0` | GPT-5.2 / GPT-5.3-Codex cache creation price |
| `REMEM_PRICE_GPT5_CODEX_CACHE_READ_PER_MTOK` | `0.175` | GPT-5.2 / GPT-5.3-Codex cached input price |

OpenAI family price overrides also support the same suffixes for `GPT55`,
`GPT54`, `GPT54_MINI`, `GPT54_NANO`, `GPT5`, and `CODEX_MINI`.

## Usage Reporting

```bash
remem usage --days 14 --weeks 8
remem usage --project /path/to/project --days 30 --weeks 12
```

The usage report reads `ai_usage_events` and renders:

- Total calls, token breakdown, and estimated cost for the selected weekly window
- Daily buckets for the selected day window
- Weekly buckets for the selected week window
- Precision summary separating provider/log usage from `text_estimate` fallback rows

Cost is intentionally labeled as estimated. Historical rows can be text
estimates or repriced rows from older schema versions; new Codex rows should use
`codex_log` from the current `codex exec --json` event stream.

## Data Cleanup

```bash
remem cleanup --dry-run --json    # Preview retention counts
remem cleanup                     # Apply cleanup
```

Cleans:
- Expired active memories: mark `stale`; keep provenance rows
- Inactive workstreams: pause after 14 days, abandon after 30 days paused
- Events: delete rows older than 30 days
- Compressed source observations: delete `status='compressed'` source rows
  90 days after the compression link was created, only if
  `compressed_observation_sources` preserves source hash and snapshot evidence
- Stale memories: archive rows older than 180 days

Retention matrix:

| Data | Retention | Cleanup behavior | Provenance requirement |
|---|---:|---|---|
| `events` | 30 days | Hard delete | None; these are low-level captured events |
| active memories with `expires_at_epoch` | Until expiry | Mark `stale` | Row remains auditable |
| stale memories | 180 days | Mark `archived` | Row remains auditable |
| workstreams | 14/30 days inactivity | Pause/abandon | Row remains auditable |
| compressed replacement observations | Indefinite | Retained | Preserve retrieval and source-summary context |
| compressed source observations | 90 days after compression link | Hard delete only when eligible | Required `compressed_observation_sources` hash + snapshot + live compressed row |
| raw archive, session summaries, candidates, edges | Indefinite by default | No cleanup in this command | Retained for audit/eval unless future policy says otherwise |

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

-- Typed graph contract for future traversal; see docs/graph-contract.md
graph_file_nodes (project_id, source_project, path, created_at_epoch, updated_at_epoch)
graph_edges (edge_type, edge_trust, from_node_kind/from_node_id, to_node_kind/to_node_id,
             source_event_ids, source_candidate_id, source_operation_id, confidence,
             reason, valid_from_epoch, valid_to_epoch)

-- Session summaries
session_summaries (memory_session_id, project, request, completed, decisions, learned,
                   next_steps, preferences, discovery_tokens, session_row_id,
                   covered_from_event_id, covered_to_event_id)

-- WorkStreams (cross-session task tracking)
workstreams (project, title, status, next_action, blockers,
             created_at_epoch, updated_at_epoch)

-- Session mapping
sdk_sessions (content_session_id → memory_session_id, project, prompt_counter)

-- Rate limiting
summarize_cooldown (project, last_summarize_epoch, last_message_hash)

-- AI call statistics
ai_usage_events (created_at_epoch, project, operation, executor, model,
                 input_tokens, output_tokens, reasoning_tokens,
                 cache_creation_tokens, cache_read_tokens,
                 raw_input_tokens, raw_output_tokens,
                 total_tokens, estimated_cost_usd,
                 usage_source, pricing_source)

-- Full-text indexes
observations_fts (title, subtitle, narrative, facts, concepts)  -- FTS5 trigram
memories_fts (title, content)                                    -- FTS5 trigram
```

## Design Decisions

- **Short-lived process model**: Each hook call = independent process, zero shared state, <6ms response, never blocks Claude Code
- **SQLite constraint compensation**: No in-memory Map dedup capability, DB tables (`summarize_cooldown`) simulate rate limiting
- **Executor-specific AI calls**: Anthropic HTTP is preferred when API credentials exist; Codex hosts can use `codex exec` with explicit model control
- **Stop hook async**: Dispatcher returns in 6ms, `std::process::Command` spawns independent worker
- **SQLite single-file + WAL**: Zero dependencies, FTS5 full-text search, WAL concurrent read/write
- **Queue batch processing**: Claude Code PostToolUse only queues (<1ms), Stop processes ≤15 events in one AI call
- **Decision priority**: Summary fields ordered decisions > completed > learned, architectural knowledge most valuable
- **Schema version control**: `PRAGMA user_version` skips repeated migration, reduces per-hook DB overhead
- **Stable project key**: `parent/dirname@hash12`, readable prefix + canonical path hash, eliminates same-name directory collisions
- **Branch-aware memories**: Memories tagged with git branch, current branch prioritized in context
- **Auto-promotion**: Session summaries automatically distilled into typed memories (decision/bugfix/preference/discovery)
- **Preference-first context**: Preferences rendered before core memories, always visible at session start
- **Explicit global scope for preferences**: Preferences stay `project`-scoped by default. Cross-project preferences require explicit `scope=global` and SessionStart global preference injection is disabled by default. Inspired by Augment's User Rules vs Workspace Rules separation
