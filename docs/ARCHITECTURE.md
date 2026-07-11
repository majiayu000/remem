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
│  search              │  │  1. extract (capture→derived)       │
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
│  captured_events → extraction_tasks → observations         │
│  memories (decision/bugfix/preference/discovery/...)       │
│  session_summaries    workstreams    FTS5 full-text index   │
│  summarize_cooldown   ai_usage_events                      │
└───────────────────────────────────────────────────────────┘
```

Codex legacy `PostToolUse(Bash)` observe hooks are treated as opt-in only:
they are skipped unless `REMEM_ENABLE_CODEX_BASH_OBSERVE=1` is set. Accepted
events still enter the coalesced capture ledger; they do not create legacy
`pending_observations` rows.

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
| `observe_flush.rs` | 609 | Legacy pending-observation flush support |
| `workstream.rs` | 581 | WorkStream tracking across sessions (auto-create + fuzzy match) |
| `mcp/server.rs` | 565 | MCP service runtime: tools, server lifecycle, tests |
| `summarize.rs` | 501 | 3-gate + background worker + session summary + compression |
| `timeline.rs` | 493 | Timeline report with monthly aggregation |
| `cli/actions.rs` | 385 | CLI command implementations and formatted output |
| `context.rs` | 368 | Context rendering: preferences + core + index + workstreams + sessions |
| `preference.rs` | 352 | Preference management: query, render, CLI ops |
| `observe.rs` | 287 | Bash filter + capture ledger writes + type checks |
| `db_pending.rs` | 261 | Legacy pending observations management |
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

### 2. Observation Capture (Claude PostToolUse → observe)

```
Tool call ──→ Type check ──→ Bash filter ──→ captured_events
               │              │
               │              └─ Skip: git status/log/diff, ls, cat,
               │                      npm install, cargo build (read-only)
               │
               └─ Accept: Claude Write/Edit/NotebookEdit/Bash/Task/Agent
                  Skip: Read, Glob, Grep, metadata-only tools
```

Accepted events store normalized host/workspace/project/session identity plus
redacted tool evidence in `captured_events`; large payloads spill to
`event_blobs`. The write also coalesces one `observation_extract`
`extraction_tasks` row per host/project/session/task kind.

### 3. Background Distillation (Stop → summarize + worker)

```
Stop hook fires
       │
       ├─ Capture session_stop → coalesced SessionRollup task
       ├─ Record immediately available citations + failure lessons
       └─ Ensure a current background worker is available
       │
       ▼
  worker claims extraction_tasks before background jobs
       │
       ├─ SessionRollup
       │    ├─ Load the captured_events range
       │    ├─ Ingest raw transcript through the Stop-captured byte boundary
       │    ├─ Finalize transcript-backed citations + failure lessons
       │    ├─ AI → semantic summary + topic segments
       │    ├─ Persist the exact event range
       │    ├─ Candidates/workstream/native-memory/user-context side effects
       │    └─ Enqueue Compress/Dream only after required side effects succeed
       │
       ├─ ObservationExtract
       │    ├─ Load captured_events + prior semantic rollup context
       │    ├─ AI → structured observations
       │    ├─ File overlap detection → mark old observations stale
       │    └─ Enqueue memory/graph/rule candidate follow-ups
       │
       ▼
  process Compress/Dream jobs
       │
       └─ Long-term compression and governed dream consolidation
```

GH684-T7 removes the legacy Summary job from the production Stop path. Stop
captures now enqueue `SessionRollup`; the rollup worker persists semantic
request, decisions, learned, next_steps, and preferences fields, then owns raw
archive ingest, summary-derived candidates, workstream updates, native-memory
sync, user-context follow-up extraction, and Compress/Dream scheduling. A
failed required side effect leaves the extraction task retryable against the
already-persisted range instead of silently completing with missing memory.
Transcript-only citation and failure-lesson side effects run after bounded raw
archive ingest on the worker; their retry errors do not suppress the other
persisted rollup side effects. Each bounded Stop with assistant evidence,
including distinct boundaries of one repeated path, snapshots the final
message hash and structured citation facts independently of the lossy prompt
budget. Retries therefore preserve long-tail and earlier-Stop citations after
the source transcript disappears. When several Stop captures coalesce into one
range, the worker drains each distinct transcript path at its widest captured
boundary and preserves pathless hook fallbacks; summary-derived candidates use
only the covered event IDs and source text from that same range. Stop payloads
that already include the final assistant message may record those idempotent
signals immediately. Versioned once-worker launch heartbeats prevent repeated
Stop hooks from spawning overlapping current workers during an old-daemon
upgrade window.
Migration v064 permanently rejects queued legacy Summary jobs and requeues any
SessionRollup lease held across the binary upgrade. Readers continue to hide
synthetic `Captured event range ...` fallback titles. The unused legacy
finalize code remains only for the later guarded-removal phase described by
GH684; it has no production caller after T7.

### 4. Context Injection (SessionStart → context)

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

### 5. Legacy Pending Queue Recovery

Runtime capture no longer writes `pending_observations`, and `session-init`
does not auto-flush that legacy queue. Old pending rows and expired legacy
processing rows have an explicit replay path instead: `remem pending
migrate-legacy` records equivalent `captured_events` with the legacy event
timestamp, enqueues `observation_extract` tasks, then marks the legacy rows
`migrated`. Rows stored with `host = unknown` require `--host
claude-code|codex-cli` so replayed evidence has a valid v2 capture identity.
Failed legacy rows stay visible through pending admin commands; retry them back
to `pending` before migration or purge them explicitly.

## Memory Lifecycle

```
Tool operations ──→ captured_events (raw capture ledger)
                         │
                         ▼ extraction_tasks (coalesced reliable work)
                         │
                         ▼ observation_extract (single AI call per range)
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

- **Incremental delta**: During extraction, inject latest 10 existing memories so AI skips duplicates
- **File overlap staleness**: When new operations overwrite old files, old observations auto-marked stale
- **Time decay**: FTS search ranked by relevance × time decay, stale observations further penalized
- **Auto compression**: Projects with >100 observations: keep newest 50, merge oldest 30 into 1-2 summaries
- **Retention cleanup**: Compression replacement observations are retained; retired
  source observations can be deleted 90 days after compression only when
  `compressed_observation_sources` still has sufficient hash/snapshot provenance
- **Failure lifecycle**: Failed pending observations, extraction tasks, replay
  ranges, and jobs carry `failure_class`, `failed_at_epoch`, and
  `archived_at_epoch`. Transient extraction/replay/job failures receive
  bounded automatic retries; permanent or exhausted failures stay visible until
  they age into archived history.

## Rate Limiting

Short-lived process model (each hook = independent process) cannot dedup via
in-memory state. remem uses SQLite state to rate-limit summary workers:

| Gate | Mechanism | Intercepts |
|------|-----------|------------|
| Gate 1 | Empty/small assistant evidence skip | Metadata-only or contentless Stop payloads |
| Gate 2 | `summarize_cooldown` after successful finalization | Same-project rapid summarize |
| Gate 3 | Last message hash dedup | Identical assistant messages |
| Worker lock | `summarize_locks` before AI call | Parallel worker races |

`summarize_cooldown` stores each project's last successful summarize time and
message hash. `summarize_locks` is the temporary per-project claim used while a
summary AI call is in flight.

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
`get_observations(source='observation')` reads current extracted-observation
details; only `pending_observations` is the legacy queue surface.

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
| `REMEM_LOG_MAX_ROTATED_FILES` | `3` | Number of rotated `remem.log.N` files to retain; accepts `0` through `100`, and `0` disables retained suffixes |
| `REMEM_LOG_LOCK_TIMEOUT_MS` | `250` | Maximum wait for the cross-process log rotation lock before append-only fallback |
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
remem cleanup --dry-run --json --archived-failures
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
- Archived failures: deleted only when `--archived-failures[=DAYS]` is supplied;
  the default explicit horizon is 90 days, and archive/purge totals are rolled
  into `failure_lifecycle_daily`

Retention matrix:

| Data | Retention | Cleanup behavior | Provenance requirement |
|---|---:|---|---|
| `events` | 30 days | Hard delete | None; these are low-level captured events |
| active memories with `expires_at_epoch` | Until expiry | Mark `stale` | Row remains auditable |
| stale memories | 180 days | Mark `archived` | Row remains auditable |
| workstreams | 14/30 days inactivity | Pause/abandon | Row remains auditable |
| compressed replacement observations | Indefinite | Retained | Preserve retrieval and source-summary context |
| compressed source observations | 90 days after compression link | Hard delete only when eligible | Required `compressed_observation_sources` hash + snapshot + live compressed row |
| failed queue rows | 14 days | Mark archived after permanent/exhausted or legacy pending failure | Row remains queryable and counted as archived history |
| archived failures | 90 days by explicit flag | Hard delete only with `--archived-failures[=DAYS]` | Aggregate history preserved in `failure_lifecycle_daily` |
| raw archive, session summaries, candidates, edges | Indefinite by default | No cleanup in this command | Retained for audit/eval unless future policy says otherwise |

## Database Schema

```sql
-- Raw capture ledger
captured_events (host_id, workspace_id, project_id, session_row_id, session_id,
                 event_id, event_type, role, tool_name, content_text,
                 content_blob_id, content_hash, created_at_epoch)

-- Reliable extraction scheduler
extraction_tasks (task_kind, host_id, workspace_id, project_id, session_row_id,
                  status, idempotency_key, cursor_event_id,
                  high_watermark_event_id, attempts, lease_owner,
                  lease_expires_epoch, failure_class, failed_at_epoch,
                  archived_at_epoch)

-- Legacy queue kept only for explicit admin migration/replay
pending_observations (session_id, project, tool_name, tool_input, tool_response, cwd,
                      created_at_epoch, status[pending|processing|failed|migrated],
                      lease_owner, lease_expires_epoch, failure_class,
                      failed_at_epoch, archived_at_epoch)

-- Failure lifecycle history for archived and purged operational failures
failure_lifecycle_daily (day_epoch, surface, failure_class, archived_count,
                         purged_count, oldest_failed_at_epoch,
                         newest_failed_at_epoch, updated_at_epoch)

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
- **Worker stderr descriptor**: Worker stderr is attached to the log file opened at worker launch; later rotation by another process may leave that descriptor writing to the already-open file until the worker exits
- **SQLite single-file + WAL**: Zero dependencies, FTS5 full-text search, WAL concurrent read/write
- **Coalesced capture processing**: Claude Code PostToolUse records capture evidence quickly; workers process coalesced extraction tasks
- **Decision priority**: Summary fields ordered decisions > completed > learned, architectural knowledge most valuable
- **Schema version control**: `PRAGMA user_version` skips repeated migration, reduces per-hook DB overhead
- **Stable project key**: `parent/dirname@hash12`, readable prefix + canonical path hash, eliminates same-name directory collisions
- **Branch-aware memories**: Memories tagged with git branch, current branch prioritized in context
- **Auto-promotion**: Session summaries automatically distilled into typed memories (decision/bugfix/preference/discovery)
- **Preference-first context**: Preferences rendered before core memories, always visible at session start
- **Explicit global scope for preferences**: Preferences stay `project`-scoped by default. Cross-project preferences require explicit `scope=global` and SessionStart global preference injection is disabled by default. Inspired by Augment's User Rules vs Workspace Rules separation
