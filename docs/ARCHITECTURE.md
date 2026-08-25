# Architecture

## System Overview

```
┌───────────────────────────────────────────────────────────┐
│              Host Hooks (Claude Code / Codex)              │
│                                                            │
│  Claude: SessionStart/UserPromptSubmit/PreToolUse/PostToolUse│
│          /Stop                                              │
│  Codex:       SessionStart/Stop                             │
│                                                            │
│  SessionStart ──────→ context       (inject memories)      │
│  UserPromptSubmit ──→ session-init  (Claude Code only)     │
│  PreToolUse(Bash) ──→ rules eval    (Claude Code only)     │
│  PostToolUse ───────→ observe       (Claude Code)           │
│  Stop ──────────────→ summarize     (3-gate + worker)      │
└──────────────┬──────────────────────┬──────────────────────┘
               │                      │
               ▼                      ▼
┌──────────────────────┐  ┌──────────────────────────────────┐
│  MCP Server (stdio)  │  │  Background Worker (detached)     │
│                      │  │                                    │
│  search              │  │  1. extract (capture→derived)       │
│  context_bundle      │  │                                    │
│  get_observations    │  │  2. compress (>100→auto merge)     │
│  timeline            │  │  3. summarize (session summary)    │
│  timeline_report     │  │  4. candidate (summary→review)      │
│                      │  │  5. compile preference rules        │
│  save_memory         │  │  6. daily lifecycle cleanup         │
│  workstreams         │  │                                    │
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
│  injection items → Context Bundle audits (hash-bound)      │
│  git_commits ↔ git_commit_sessions    ai_usage_events       │
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

Only successful explicit `git commit` calls produce linkable Git evidence.
Claude PostToolUse reads the successful Bash result; Codex Stop reads the
byte-bounded transcript and pairs shell calls with successful outputs. The
resolved metadata for each proven SHA is stored in `captured_event_commits`
before database open/spill. Deterministic worker phases consume only the exact
claimed event range, key links by `session_row_id`, and never infer a commit
from an ordinary Stop event or worker-time `HEAD`.

Codex Stop also materializes timestamped genuine transcript conversation turns
as deterministic captured `message` events before the `session_stop` row in
the same capture batch. User turns retain `user_prompt` trust, while assistant
turns and the Stop row remain `external_content` because flattened assistant
text cannot prove local provenance; meta/XML control turns are excluded from
trusted identities. Git branch state is resolved once before the batch, and
transcript-derived message events share the rollup prompt's 128-message/64 KiB
aggregate budget.

## Module Map

This ownership map is intentionally non-exhaustive. It includes primary
product-domain roots, reusable root-level owners with production callers in
at least three architectural subsystems, and owners of cross-cutting contracts
explicitly named in the data flow. It excludes executable/module-registration
glue, test-only helpers, and one- or two-subsystem implementation details.
Entries are bounded routing hints, not exhaustive or exclusive ownership
claims: a directory row describes its primary domain, while a cross-cutting
child may be routed separately when another flow owns its contract. Do not
hand-maintain line counts here; source sizes are enforced by
[`scripts/ci/check_file_size.py`](../scripts/ci/check_file_size.py) and should
be inspected from the current checkout when needed. New SQLite migrations are
one concern each; see
[`docs/maintenance/migration-discipline.md`](maintenance/migration-discipline.md).

| Area | Responsibility |
|------|----------------|
| `hook_cli.rs`, `bin/remem_hook.rs` | Slim hook entry: only `context` / `session-init` / `observe` / `summarize`; builds without `eval` and `local-onnx`. Install prefers an executable sibling `remem-hook` for those commands and leaves `rules eval` plus MCP on full `remem` |
| `adapter/`, `observe/`, `cursor_hook/` | Host hook parsing, capture filtering, capture-specific spill serialization/replay, and capture-ledger writes |
| `adapter/redaction.rs` | Cross-cutting sensitive-evidence redaction shared by observe capture, SessionRollup (including Cursor snapshots), and summarize capture; general secret, token, and URL-userinfo sanitization, plus header-key redaction and malformed-payload fallback specific to bounded hook-payload previews |
| `identity.rs`, `project_id.rs`, `project_alias.rs` | Hook-host and capture-identity type contracts, canonical project-root resolution, and alias-governed canonical writes/alias-aware reads |
| `spill_queue.rs` | Shared cross-process spill locking/appends, atomic replay claims, failed-record recovery, and orphan restoration |
| `git_util.rs` | Bounded Git subprocess execution and cleanup, repository-root and commit-metadata resolution, metadata sanitization, and shared Git evidence types/parsers |
| `git_evidence.rs`, `captured_git.rs`, `git_trace.rs`, `git_trace/` | Successful-commit evidence extraction, exact claimed-event-range linking, durable commit/session persistence, poisoning-gated trace exposure, and commit/session lookup |
| `hook_integrity.rs`, `hook_integrity/` | Canonical host-hook specifications and command parsing, installed-hook integrity evaluation/removal, and executable consistency checks for context warnings, doctor, and install |
| `atomic_file.rs` | Permission-preserving atomic file publication with unique sibling temp files, durable file/directory synchronization, final-target symlink resolution, and cross-platform replacement |
| `build_info.rs` | Canonical package/schema version labels shared by CLI, API/MCP, doctor, migrations, and persisted procedure metadata |
| `perf.rs` | Shared phase-timing capture and formatting for context loading, retrieval, summarization, evaluation, and CLI diagnostics |
| `db/`, `migrate/`, `migrations/` | SQLite/SQLCipher schema and connection policy, encrypted spill payloads, migrations, read/write helpers, and job, extraction-task, and frozen-legacy state |
| `worker.rs`, `worker/`, `extraction_worker.rs`, `maintenance/` | Background dispatch, worker singleton and heartbeats, job and extraction-task lease claims/recovery, timeout/retry transitions, task execution, idle legacy-pending migration, and lifecycle cleanup |
| `ai.rs`, `ai/`, `runtime_config.rs`, `runtime_config/` | AI executor dispatch, provider/CLI execution and usage accounting, plus host/profile/model resolution and runtime configuration. SessionStart budgets live in `[context]`; USD cost overrides live in `[pricing]` |
| `summarize.rs`, `summarize/` | Stop-hook payload intake, capture-ledger enqueue, summary-specific spill serialization/replay, once-worker launch, active Compress processing, and compatibility-only legacy Summary parsing/finalization |
| `session_rollup/`, `observation_extract.rs`, `observation_extract/` | Production session-summary generation and persistence, required side effects, and tool-event observation extraction and persistence |
| `memory_candidate.rs`, `memory_candidate/`, `graph_candidate/` | Governed memory and graph candidate generation, source/evidence validation, review/quarantine, and promotion |
| `user_context.rs`, `user_context/` | Governed user-context candidate and claim extraction, review/promotion, profile summaries, source-attributed recall, and retention/usage policy |
| `ingest/`, `memory/raw_archive.rs`, `memory/raw_occurrence.rs`, `memory/raw_query.rs`, `memory/raw_reconcile.rs`, `memory/raw_transcript.rs` | Transcript discovery and parsing, identity-ledger and occurrence ingestion, raw-archive persistence, typed/query-bounded raw reads, and aggregate reconciliation |
| `memory/` (including `memory/preference.rs` and `memory/preference/`), `workstream/`, `truth/` | Curated memory storage, formatting/deduplication, preferences, workstream continuity, and lifecycle/current-truth projections |
| `context/`, `context_bundle/`, `retrieval/`, `retrieval_router/` | SessionStart loading/rendering, optional Claude native-memory mirror rendering/sync, bundle audit, lexical/vector search and fusion, and intent-aware retrieval planning |
| `timeline.rs`, `timeline/` | Aggregated project queries and structured/Markdown report generation for the `timeline_report` MCP flow |
| `db/query/timeline.rs` | Chronological observation-neighborhood queries for the `timeline` MCP flow |
| `dream/`, `rules/`, `eval/` | Memory consolidation, compiled preference rules, benchmark and policy evaluation gates |
| `eval/coding_bench/` | GH931 coding-agent matrix planning, closed raw-event capture projection, isolated production capture/drain/SessionStart binding, target-blind curator verification, runner isolation/scoring, directional report assembly, and the zero-dispatch signed live-approval/trust-root/supervisor-attestation gate |
| `log.rs`, `log/` | File logging with cross-process locking/rotation and append fallback, stderr mirroring, private permissions, timing, worker-stderr preparation, and health snapshots |
| `cli/`, `mcp/`, `api/`, `doctor/`, `install/` | User-facing commands, MCP/REST surfaces, diagnostics, and host configuration |

### Enforced dependency direction

The exhaustive top-level ownership contract lives in
[`docs/specs/GH969/TECH.md`](specs/GH969/TECH.md#target-module-direction).
Dependencies target inward from adapters through application,
memory/retrieval, storage, and foundation; evaluation and doctor are the
outermost evidence/diagnostic layer. Current reverse edges are explicit debt,
not approved architecture: their exact source sites and the largest cyclic
component are pinned in
[`module-dependency-baseline.json`](specs/GH969/module-dependency-baseline.json).

Run the same no-expansion check used by preflight and CI with:

```bash
python3 scripts/ci/check_module_dependencies.py --base origin/main
python3 scripts/ci/check_module_dependencies.py --self-test
```

The check fails on an unclassified root, a new reverse edge, a new site on an
accepted reverse edge, stale baseline debt that should be removed, or growth of
the largest cyclic component. Temporary exceptions require an owner,
rationale, tracking issue, and decision date.

## Data Flow

Current-context reads first share the deterministic, read-only trust/visibility
projection in `src/truth/visibility.rs`. It classifies historical active rows
without rewriting them and fails closed when proof is unknown. The default
Context Bundle/SessionStart path then applies the bounded CurrentTruth v1
projection released in v0.6.81 from `src/context_bundle/current_truth.rs`:
selected Core claims carry stable projection/evidence references,
contradictions produce audited abstentions, and a projection failure cannot
revive newest-wins Core output. The `legacy` render mode rolls back Context
Bundle relevance/audit only and retains this CurrentTruth-governed Core path.
This is a production v1 precursor, not completion of GH933's broader Phase B,
and not the typed/history-replayable v2 writer and migration contract that
remains pending there.

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

Cursor (GH-823) enters this same ledger through a separate strict boundary
(`src/cursor_hook/` + `src/observe/cursor.rs`): a bounded 1 MiB stdin reader,
fail-closed payload validation against the Cursor 3.12.17 evidence,
`user_email` PII removal, `tool_use_id` as the canonical per-call event key,
and the existing `captured_events.event_type = "cursor_tool_failure"` text
discriminator for the observed failed-Read path. MCP-specific Cursor events
stay unregistered (generic ownership), and `session-init` remains
unsupported/fail-closed on Cursor. Stop-time transcript capture (GH-825) and
the install surface (GH-824) are described below.

#### Cursor host data flow (GH-823/824/825)

```
Cursor hook payload (stdin, bounded 1 MiB)
       │
       ├─ observe: strict fail-closed parse of the verified generic
       │  tool event (tool_use_id = per-call identity, user_email
       │  removed pre-capture) ──→ captured_events / event_blobs,
       │  with spill-and-replay when the DB is unavailable
       │
       └─ stop: full Stop validation (status ∈ {completed, aborted},
          canonical key = session_id:generation_id:loop_count)
               │
               ├─ Stop-time transcript snapshot (bounded read); every
               │  failure maps to an explicit degraded/<reason> marker
               │  in the durable session_stop payload — payload-only
               │  fallback, never a silent drop
               └─ enqueue the same SessionRollup path as Claude/Codex
```

`remem summarize --host cursor` wires the GH-825 snapshot into the shared
rollup worker; the worker never reopens the original transcript path, and
capture fidelity (`full` vs `degraded/<reason>`) stays auditable from the
ledger.

The install surface (GH-824, `src/install/cursor_config/` +
`src/install/hosts/cursor.rs`) owns the user-level `~/.cursor/hooks.json`
and `~/.cursor/mcp.json` through a strict whole-document parser, a
read-only preflight, and a staged-apply coordinator with compensating
rollback plus an install receipt for exact structural ownership. Contract
v1 registers exactly one MCP component and no hook entries: the observe and
summarize capability gates are closed until the corresponding runtime
policies are approved, so installing does not enable automatic Cursor
capture. `sessionStart` injection is blocked on the evidenced Cursor
version and never installs. The platform gate approves the hook command
renderer only on macOS/Linux; Windows fails closed (explicit error for
`--target cursor`/`all`, skip diagnostic for `--target auto`).

`remem doctor` reports Cursor as separate dimensions instead of one
"installed" boolean — `detected`, `configured`, `configured_mode`,
`malformed`, `partial_state`, `drift`, `collision`, per-capability
`effective` lines — plus the fixed
`hook_failure_policy: host_continues` and
`session-init: not supported on cursor` lines.

### 3. Background Distillation (Stop → summarize + worker)

```
Stop hook fires
       │
       ├─ Capture session_stop → coalesced SessionRollup task
       ├─ Record immediately available citations + failure lessons
       └─ Ensure a current background worker is available
       │
       ▼
  worker claims due lifecycle cleanup first, then extraction_tasks before
  ordinary background jobs
       │
       ├─ SessionRollup
       │    ├─ Load the captured_events range
       │    ├─ Idempotently link every proven Git commit in that range
       │    ├─ Resolve path-stable transcript identity and claim
       │    ├─ Ingest raw occurrences through the Stop-captured byte boundary
       │    ├─ Finalize transcript-backed citations + failure lessons
       │    ├─ AI → semantic summary + topic segments
       │    ├─ Persist the exact event range
       │    ├─ Candidates/workstream/native-memory/user-context side effects
       │    └─ Enqueue Compress/Dream only after required side effects succeed
       │
       ├─ ObservationExtract
       │    ├─ Load captured_events + prior semantic rollup context
       │    ├─ Idempotently link every proven Git commit in that range
       │    ├─ AI → structured observations
       │    ├─ File overlap detection → mark old observations stale
       │    └─ Enqueue memory/graph/rule candidate follow-ups
       │
       ├─ When no current extraction task is ready
       │    └─ Admit one rate-limited legacy drain batch into extraction_tasks
       │
       ▼
  process Compress/Dream jobs
       │
       ├─ Long-term compression and governed dream consolidation
       │
       └─ When current work is idle, admit one retrieval-enrichment batch
            ├─ four rows maximum
            ├─ once worker: one batch per process
            ├─ daemon: one batch per 60 seconds
            └─ pending → ready, or exhausted after three failures
```

A once worker shares a process-local budget of four potential AI work items
across extraction tasks, Compress/Dream jobs, and retrieval enrichment, and
stops admitting new work after 180 seconds. The budget is checked between
items; an already-running provider call keeps its surface-specific timeout.
Daemon workers remain long-running, with the retrieval-enrichment interval
providing the background rate limit.

Dream treats all generated surfaces as untrusted; every decision rechecks snapshot, TTL, current-state, and suppression under its write lock, and poisoned output becomes an atomic quarantine artifact.
Clean output stays external trust and cannot reuse an unreviewed target; review tokens bind the exact source set. See [the poisoning contract](specs/memory-poisoning-defense/TECH.md).

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

Migration v071 separates a transcript's path-stable local identity from its
filename and metadata claims. Stop and batch ingestion share the same
metadata-first probe; the batch path persists the complete claim set before it
mutates raw rows, keeps conflicts sticky, and upgrades legacy rows to stable
transcript occurrence ordinals without losing repeated identical turns.
`raw_messages.event_time_source` distinguishes transcript event time,
ingest-time fallback, and legacy-unknown provenance.

The raw query path is read-only and schema-validated:

```text
raw search / raw sessions
  -> open_db_read_only_current
  -> current-schema and drift validation
  -> bounded SQL query

raw reconcile
  -> discover with ingest-sessions root/subagents rules
  -> validate current identity-ledger mtime/size tuple
  -> stream only window-intersecting or missing-time transcript snapshots
  +  query matching raw occurrence identities
  -> aggregate-only parity report
```

Reconciliation keeps paths, projects, session IDs, content, and hashes inside
the process and encrypted local database. Public JSON contains counts and
fixed policy/window metadata only.
Migration v068 makes follow-up scheduling an exact-range transaction. New
ranges persist their Compress job id and one structured Dream outcome with its
referenced job id. Exact ranges created before v068 are marked
`legacy_unknown`; retries report manual reconciliation at error level and do
not infer replacement work from terminal job history. The same default applies
when an already-running pre-v068 worker inserts its range after migration;
current writers explicitly initialize new ranges and v068 requeues old
processing leases so the upgraded worker owns completion.
Migration v064 permanently rejects queued legacy Summary jobs and requeues any
SessionRollup lease held across the binary upgrade. Readers continue to hide
synthetic `Captured event range ...` fallback titles. The unused legacy
finalize code remains only for the later guarded-removal phase described by
GH684; it has no production caller after T7.

### Compiled Preference Rules (worker → artifact → Claude PreToolUse)

```text
eligible preferences + reinforcement + suppressions + rule overrides (SQLite)
       │
       ├─ lifecycle jobs and periodic convergence sweep
       ▼
background worker compiler
       │
       └─ atomic derived artifact: compiled_rules/<project-hash>.json
                                      │
                                      ▼
                         Claude PreToolUse(Bash)
                                      │
                         visible warn / explicit block
```

Rule compilation is disabled by default through
`rule_compilation.enabled`. SQLite is the source of truth; only the worker
writes the versioned artifact, while hook evaluation is deterministic,
read-only, and performs no LLM, network, or database write. `remem rules
list|disable|enable|set-action` reads provenance or persists an override, then
the worker rebuild makes the effective action/disabled state visible without a
host restart.

Claude Code installs a pre-execution Bash evaluator, so `warn` can be visible
before execution and `block` can be honored only after explicit per-rule user
opt-in. Claude `PostToolUse` remains capture-only. Codex has no supported
pre-execution command hook, so command enforcement is reported as unsupported
and Codex block-mode claims are rejected. Missing, corrupt, or unsupported
artifacts fail open and emit error-level diagnostics.

Doctor reports enabled state, artifact presence/validity and rule count,
compile status/time/error, the latest project/global evaluation error, and
Claude/Codex enforcement capability without exposing rule payloads. GH-671
remains open: #813 still owns the exact global `user` / `user:default` /
no-target eligibility correction and exhaustive closed-policy matrix.

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
  Load bounded memory, lesson, and session candidates
       │
       ├─ Branch-aware: current branch first, then main, then others
       ├─ Score-based: decision > bugfix > architecture > discovery
       ├─ Freeze Core section unchanged: top 6 scored, 200-char preview
       ├─ Score Lessons + non-Core Index + Sessions against the implicit query
       ├─ Apply one global relevance k (default 1), then section budgets
       └─ Keep k=0 as the legacy-selection rollback
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

Bundle-backed emissions persist the sealed, payload-free `ContextAudit` and
its SHA-256 in the same transaction as `context_injection_items`. Both share
`injection_run_id`; verified readers recompute canonical JSON and compare all
denormalized counts, versions, and hashes before exposing benchmark metadata.
Coding-bench remem conditions consume that exact injection row, embed its
payload-free canonical audit in the run contract, and independently recompute
the hash. Its v1 verifier rejects unknown audit fields and derives selection
counts from the embedded entries. A domain-separated artifact binding
additionally covers the `injection_run_id` and audit hash. Control conditions
carry an explicit `not_applicable` status instead of an empty or synthetic
audit.

### 5. Legacy Pending Queue Recovery

Runtime capture no longer writes `pending_observations`, and `session-init`
does not auto-flush that legacy queue. The deleted enqueue/claim API stays
deleted. Ordinary workers instead expose a drain-only migration bridge for
residual rows, and consider it only after the current extraction worker finds
no ready task. After a store has no residual auto-recoverable rows (including
delayed retries and active leases), v085
persists `legacy_surface_state.state = exhausted` and workers skip the bridge
until a residual row is reintroduced. Guarded table drop remains remem 0.7.0.

A once worker admits at most one batch in its process lifetime. A daemon admits
at most one batch every 60 seconds. Each batch contains at most 25 oldest rows
with a known `claude-code` or `codex-cli` host: pending rows, expired processing
rows, due transient failures, and controlled historical archived transient
failures. Permanent and unknown-host rows are not automatic candidates. If
current work appears during preflight, a zero-progress yield keeps the
once/interval admission available after that work drains; a partial-progress
yield consumes the admission and preserves the per-process/per-interval cap.

For each selected row, one transaction records the deterministic legacy
`captured_event`, enqueues its current `ObservationExtract` task, marks the
legacy source `migrated`, and clears its old lease, retry, failure, and archive
state. A repeated attempt converges on the same event/task. A shared or
otherwise failed replay rolls back current-pipeline writes, records exponential
backoff capped at 900 seconds, and stops the batch. The bridge never infers a
row-local permanent classification from a shared replay failure. Selection or
a failed attempt never unarchives a row, and the bridge never deletes data.
Before an immediate write transaction starts, automatic and exact replay
snapshot their source row, while manual batch migration snapshots every
selected row; all paths resolve optional Git branch metadata before locking.
The transaction reloads and revalidates each candidate before writing, and
capture receives the explicit precomputed branch value, including an explicit
no-branch result. No Git subprocess runs while the SQLite write lock is held.

Doctor reports due and deferred known-host archived transient rows separately.
A deferred row remains a capture-liveness failure with its earliest retry epoch,
but does not receive immediate once-worker guidance. Archived failed rows
excluded from the automatic bridge are `admin-required`; the check queries
those candidates independently of the global failed-row listing and prints a
bounded oldest-first set with real IDs, stored hosts, failure classes, archive
times, and concrete preview/apply commands. A stored `host = unknown` prints
both explicit `--host claude-code|codex-cli` choices. The exact command rejects
rows that are not both failed and archived, replays only the requested ID in
one transaction, and clears failure/archive state only after the current event
and task commit. Any failure rolls back current-pipeline writes and preserves
the source row.

## Memory Lifecycle

```
Tool operations ──→ captured_events (raw capture ledger)
                         │
                         ▼ extraction_tasks (coalesced reliable work)
                         │
                         ▼ observation_extract (single AI call per range)
                  observations (structured memory)
                         │
                         ▼ claim-level support + closed risk/type gate
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
- **Candidate promotion boundary**: each candidate sentence must be supported
  by an eligible source observation with matching polarity/modality. Directly
  supported observation-path failure-and-recovery lessons are the only lesson
  subtype that can auto-promote; preference/procedure types remain fail-closed.
  Canonicalized credential-token variants, auth/authz controls, affirmative
  destructive actions, generic imperatives, outer meta-negation, and ambiguous
  modals are review-gated independently of the model's risk label. Secret
  phrases and known instruction patterns are blocked before memory creation,
  and all pending decisions retain an explicit block reason.
- **File overlap staleness**: When new operations overwrite old files, old observations auto-marked stale
- **Time decay**: FTS search ranked by relevance × time decay, stale observations further penalized
- **Auto compression**: Projects with >100 observations: keep newest 50, merge oldest 30 into 1-2 summaries
- **Retention cleanup**: Compression replacement observations are retained; retired
  source observations can be deleted 90 days after compression only when
  `compressed_observation_sources` still has sufficient hash/snapshot provenance
- **Failure lifecycle**: Failed pending observations, extraction tasks, replay
  ranges, and jobs carry `failure_class`, `failed_at_epoch`, and
  `archived_at_epoch`. Transient extraction/replay/job failures receive
  bounded automatic retries. Eligible legacy pending rows use the idle-only,
  25-row drain bridge; once workers admit one batch and daemons admit one batch
  per 60 seconds, except a zero-progress yield retains that admission. Deferred
  archived transient rows remain doctor-visible with their next retry; archived
  permanent or unknown-host legacy rows are `admin-required` and use exact
  `recover-archived`. No automatic path deletes them.

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
- **Timeouts**: Ordinary AI calls use the shared 90s deadline; retrieval
  enrichment has a 120s outer attempt deadline below its 300s lease, and
  durable worker jobs have a 420s per-job deadline below their 480s lease.
- **Retrieval-enrichment admission**: schema v083 marks incomplete pre-upgrade
  rows `deferred`; only new or canonically changed `pending` rows enter the
  automatic four-row lane. One-shot workers admit one batch, daemons admit one
  batch per 60 seconds, and the third failure becomes `exhausted`.
- **Unified prompts**: summarize, session rollup, observation extract, memory candidate, compress, and dream all resolve through the same host/profile config
- **Usage ledger**: `ai_usage_events` stores model, operation, token breakdown, usage source, pricing source, and estimated USD cost
- **Codex credit pricing**: GPT-5.6 Luna/Sol/Terra rows keep token counts but
  use `unknown_pricing` for USD unless an explicit operator override is set.
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

The stdio MCP server exposes 15 tools:

- Retrieval and context compilation: `current_state`, `search`,
  `recall_user_context`, experimental `context_bundle`, `timeline`, `search_raw`,
  and `list_raw_sessions`.
- Detail and trace: `get_observations`, `lookup_commit`, and
  `commits_for_session`.
- Mutation and reporting: `save_memory`, `govern_memory`, `timeline_report`,
  `workstreams`, and `update_workstream`.

All descriptors carry explicit title/read-only/destructive/idempotent/open-world
annotations. Fourteen JSON tools preserve their existing text content and add
object-rooted `outputSchema` plus matching `structuredContent`; legacy arrays
use named structured envelopes. `timeline_report` remains Markdown-only.

Recommended workflow: `search(query)` → find relevant IDs → `get_observations(ids)` for full content.
`get_observations(source='observation')` reads current extracted-observation
details; only `pending_observations` is the legacy queue surface.

`save_memory` behavior:
- Dual-write by default: SQLite memory + local Markdown (`~/.remem/manual-notes/<project>/...md`)
- Custom local path via `local_path` parameter
- Optional `idempotency_key` binds retries to the same payload; a conflicting replay fails closed
- When user asks to "save a document", write project-local file first, then `save_memory` as long-term backup

All production paths that leave a curated memory `active` pass through
`memory::activation`. Schema v86 records an immutable request identity, route,
actor, trust class, provenance, payload digest, poisoning verdict, exact
supersede set, resulting memory ID, and a digest recomputed from the stored
title/content/type/topic/files/evidence payload in `memory_activation_requests`.
Schema v87 adds result trust to that ledger; v88 upgrades already-recorded v86
receipts from v87's unknown marker using only the immutable source-trust
postcondition and preserves receipt rowids for replay chronology.
The boundary compares those stored fields with the reviewed request and repeats
the poisoning check as a postcondition. Backup best-effort import is recorded
as governed `backup_import`; only identity-preserving restore paths use
`exact_recovery`. The
active mutation and its audit record share one savepoint, so validation or
audit failure rolls back the whole activation. The preflight/CI bypass guard
rejects newly introduced direct active-memory SQL or raw helper calls.

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
| `REMEM_CONFIG` | `~/.remem/config.toml` | Runtime config file for memory-AI host/profile policy and `[context]` budgets |
| `ANTHROPIC_API_KEY` | - | Required for HTTP mode (also supports `ANTHROPIC_AUTH_TOKEN`) |
| `REMEM_DEBUG` | - | Enable debug logging |
| `REMEM_CONTEXT_TOTAL_CHAR_LIMIT` | `[context].total_char_limit` (`12000`) | Env escape hatch for the SessionStart total character cap |
| `REMEM_CONTEXT_CANDIDATE_FETCH_LIMIT` | `[context].candidate_fetch_limit` (`120`) | Env escape hatch for candidate fetch before section selection |
| `REMEM_CONTEXT_MEMORY_INDEX_LIMIT` | `[context].memory_index_limit` (`50`) | Env escape hatch for the main memory index item cap |
| `REMEM_CONTEXT_OBSERVATIONS` | same as memory index | Deprecated alias for `REMEM_CONTEXT_MEMORY_INDEX_LIMIT` |
| `REMEM_CONTEXT_MEMORY_INDEX_CHAR_LIMIT` | `[context].memory_index_char_limit` (`4000`) | Env escape hatch for the main memory index character budget |
| `REMEM_CONTEXT_CORE_ITEM_LIMIT` | `[context].core_item_limit` (`6`) | Env escape hatch for the core item cap |
| `REMEM_CONTEXT_CORE_CHAR_LIMIT` | `[context].core_char_limit` (`3000`) | Env escape hatch for the core character budget |
| `REMEM_CONTEXT_SESSION_COUNT` | `[context].session_count` (`5`) | Env escape hatch for session summaries shown |
| `REMEM_CONTEXT_RELEVANCE_K` | `[context].relevance_k` (`1`) | Env escape hatch for the global relevant-item cap; `0` restores legacy selection |
| `REMEM_CONTEXT_SELF_DIAGNOSTIC_LIMIT` | `[context].self_diagnostic_limit` (`2`) | Env escape hatch for the self-diagnostic cap |
| `REMEM_CONTEXT_PREFERENCE_PROJECT_LIMIT` | `[context].preference_project_limit` (`20`) | Env escape hatch for the project preference query cap |
| `REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT` | `[context].preference_global_limit` (`0`) | Env escape hatch for the global preference query cap; compiled default is 0 (disabled) |
| `REMEM_CONTEXT_PREFERENCE_CHAR_LIMIT` | `[context].preference_char_limit` (`1500`) | Env escape hatch for the preference character budget |
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

The worker also probes for one database-global automatic cleanup. SQLite owns
the active identity and 24-hour completed-attempt cooldown, so daemon restarts
and repeated `worker --once` processes do not duplicate the run. A dedicated
cleanup claim prevents extraction backlog starvation. Cleanup effects, the
successful `maintenance_runs` ledger row, and the job's `done` transition
commit in one immediate transaction; failures roll back before a bounded
failure row is recorded.

Cleans:
- Expired active memories: mark `stale`; keep provenance rows
- Inactive workstreams: pause after 14 days, abandon after 30 days paused
- Events: delete rows older than 30 days only when explicitly classified
  `ephemeral`; preserve audit rows and every API-referenced event
- Compressed source observations: delete `status='compressed'` source rows
  90 days after the compression link was created, only if
  `compressed_observation_sources` preserves complete source hash/snapshot
  evidence, an active replacement remains, and no fact references the source
- Stale memories: archive rows older than 180 days
- Archived failures: deleted only when `--archived-failures[=DAYS]` is supplied;
  the explicit horizon must be positive, defaults to 90 days when the flag has
  no value, and archive/purge totals are rolled into
  `failure_lifecycle_daily`. Automatic cleanup cannot select this operation.

Retention matrix:

| Data | Retention | Cleanup behavior | Provenance requirement |
|---|---:|---|---|
| `events` with `retention_class='ephemeral'` | 30 days | Hard delete | Governance/audit and API-referenced rows are retained |
| `events` with `retention_class='audit'` | Indefinite | No retention delete | Governance provenance remains restorable |
| Context Bundle audit summaries | `REMEM_CONTEXT_GATE_RETENTION_DAYS` | Hard delete expired summary rows; never update surviving rows | Canonical audit JSON/hash remains linked to item rows by `injection_run_id` during retention |
| active memories with `expires_at_epoch` | Until expiry | Mark `stale` | Row remains auditable |
| stale memories | 180 days | Mark `archived` | Row remains auditable |
| workstreams | 14/30 days inactivity | Pause/abandon | Row remains auditable |
| compressed replacement observations | Indefinite | Retained | Preserve retrieval and source-summary context |
| compressed source observations | 90 days after compression link | Hard delete only when eligible | Matching canonical `observation-v2` hash/snapshot revalidated across the AI window (or an exact legacy v1 link upgraded transactionally to v2), independently checked status, known schema, active replacement, and no fact reference |
| failed queue rows | 14 days | Mark permanent/exhausted current-pipeline rows archived; legacy `pending_observations` transient rows stay visible for drain/admin recovery, a known-host archived historical transient may enter the bounded drain, and an archived permanent/unknown-host row requires exact `recover-archived` | Archive/failure state stays intact until one atomic migration succeeds |
| archived failures | 90 days by explicit flag | Hard delete only with `--archived-failures[=DAYS]` | Aggregate history preserved in `failure_lifecycle_daily` |
| raw archive, session summaries, candidates, edges | Indefinite by default | No cleanup in this command | Retained for audit/eval unless future policy says otherwise |

## Database Schema

```sql
-- Raw capture ledger
captured_events (host_id, workspace_id, project_id, session_row_id, session_id,
                 event_id, event_type, role, tool_name, content_text,
                 content_blob_id, content_hash, created_at_epoch,
                 reference_time_epoch)

-- One event can prove multiple explicit commits; snapshots are not linkable
captured_event_commits (event_row_id, sha, metadata_json, evidence_kind,
                        evidence_locator)

-- Capture-time Git provenance; session_row_id prevents cross-host raw-ID collisions
git_commits (project, repo_path, sha, short_sha, branch, message, changed_files)
git_commit_sessions (commit_id, session_row_id, session_id, memory_session_id,
                     source, linked_at_epoch)

-- Reliable extraction scheduler
extraction_tasks (task_kind, host_id, workspace_id, project_id, session_row_id,
                  status, idempotency_key, cursor_event_id,
                  high_watermark_event_id, attempts, lease_owner,
                  lease_expires_epoch, failure_class, failed_at_epoch,
                  archived_at_epoch)

-- Frozen legacy queue: bounded worker drain plus explicit admin fallback;
-- no current enqueue/claim API
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

-- Typed graph contract with bounded trusted-edge search traversal; see docs/graph-contract.md
graph_file_nodes (project_id, source_project, path, created_at_epoch, updated_at_epoch)
graph_edges (edge_type, edge_trust, from_node_kind/from_node_id, to_node_kind/to_node_id,
             source_event_ids, source_candidate_id, source_operation_id, confidence,
             reason, valid_from_epoch, valid_to_epoch)

-- Session summaries
session_summaries (memory_session_id, project, request, completed, decisions, learned,
                   next_steps, preferences, discovery_tokens, session_row_id,
                   covered_from_event_id, covered_to_event_id)

-- Per-item injection evidence plus payload-free Context Bundle audit
context_injection_items (injection_run_id, host, project, session_id, item_kind,
                         item_id, channel, status, drop_reason, injected_at_epoch)
context_bundle_audits (injection_run_id, bundle_schema_version,
                       plan_schema_version, policy_version,
                       relevance_policy_version, plan_hash, audit_hash,
                       degraded_mode, candidates_considered, selected_count,
                       dropped_count, token_budget, token_estimate,
                       truncation_reason, audit_json, created_at_epoch)

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
