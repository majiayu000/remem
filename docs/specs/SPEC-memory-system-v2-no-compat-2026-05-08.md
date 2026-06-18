---
spec: memory-system-v2-no-compat
status: proposed
date: 2026-05-08
owner: remem
compatibility: breaking
supersedes:
  - SPEC-observation-drain-scheduler-2026-05-05.md
  - SPEC-raw-archive-vs-curated-memory-2026-04-22.md
---

# remem Memory System v2 Spec（不考虑向后兼容）

## 0. Executive Summary

remem v2 的核心判断：

> 自动捕获是正确方向；错误的是把 raw event queue 当成 memory system。

v2 不再把 `pending_observations` 视为长期记忆入口，而是拆成五层：

1. **Capture Log**：快速、AI-free、append-only，保证发生过的事可追溯。
2. **Extraction Tasks**：可靠 worker/scheduler 队列，负责把 raw events 转成可用中间产物。
3. **Derived Knowledge**：session summaries、observations、memory candidates、rule candidates。
4. **Curated Memory**：真正进入长期上下文的高信号 memory，有 provenance、confidence、staleness 和 review 状态。
5. **Context Compiler**：按预算把 rules、recent work、curated memory、raw fallback 和 code index 组合进 agent context。

这是一版破坏性重构：

- 不保留旧 `pending_observations` / `jobs` 的运行时兼容。
- 不支持 `host = unknown` 的兼容 claim。
- 不保证旧 MCP response shape 完全兼容。
- 不在新 worker 中读取旧队列表。
- 旧 DB 只允许通过显式 backup/export/import 进入 v2，不做隐式迁移。

## 1. Problem Statement

当前设计的问题不是单个 batch 太小，而是 memory boundary 错了。

### 1.1 Raw event 与 memory 混淆

工具事件、Bash 输出、Stop hook payload、chat transcript 都是 raw evidence。它们可以用于提炼 memory，但本身不是 durable memory。

旧设计的隐含假设是：

```text
raw event -> pending_observations -> observation -> memory/search context
```

这个模型会导致：

- 低价值事件和高价值知识争抢同一条队列。
- 工具噪音越多，memory 质量越差。
- 队列积压时，真正重要的 summary/memory 也被拖住。
- 没有明确的 promotion 标准，容易过度记忆或漏记。

### 1.2 Consumer 语义太弱

旧 worker 的实际语义接近：

```text
Stop hook enqueues work
worker --once handles a small batch
future Stop hook maybe wakes it again
```

这不是可靠队列系统。正确语义应该是：

```text
capture is append-only
scheduler continuously discovers ready work
worker advances identities under explicit budgets
leases recover automatically
status/doctor show real queue health
```

### 1.3 Identity 不完整

所有 capture、task、memory 都必须有明确 identity：

```text
host + workspace + project + session_id + turn_id/event_id
```

只靠 `session_id` 会混淆 Claude Code / Codex / Cursor-like adapters，也会混淆同名 session 在不同 workspace/project 下的语义。

### 1.4 缺少 backpressure

没有队列上限、事件大小上限、AI budget、priority isolation 和 drop/compact policy 时，任何高频 Bash 或长工具输出都会制造无界 backlog。

### 1.5 缺少 review/governance

Cursor/Augment 的公开设计都把“长期记住什么”作为可见、可控的层：

- Cursor memories 是自动生成的 rules，background-generated memory 保存前需要用户批准。
- Augment Memory Review 允许 approve/edit/discard。

remem 不能依赖 Codex 主动调用 `save_memory`，但也不能把每个 raw event 自动升级为长期 memory。v2 需要自动捕获 + 自动提候选 + 可审查/可纠错的 durable memory。

## 2. Goals

- 自动捕获仍然是主路径，不依赖 agent 自觉调用 `save_memory`。
- Hook 快速、AI-free，不能因为 LLM 或 worker 慢阻塞用户。
- Raw evidence 与 curated memory 明确分层。
- Worker/scheduler 能长期稳定 drain，不依赖未来 Stop hook。
- 所有队列处理都有 leases、retry、backoff、idempotency 和 progress metrics。
- 长期 memory 有类型、证据、置信度、staleness、scope 和 review 状态。
- Context 注入受预算约束，不把 raw archive 直接塞进 prompt。
- 支持 Codex 和 Claude Code，但通过 host identity 隔离。
- 支持破坏性 schema reset，换取更简单、更正确的数据模型。

## 3. Non-Goals

- 不做旧 DB 原地兼容迁移。
- 不让 v2 worker 读取旧 `pending_observations` / `jobs`。
- 不保留 `unknown` host runtime path。
- 不把所有 tool output 永久记忆化。
- 不在 capture hook 调用 AI。
- 不新增微服务；仍保持 Rust single-binary。
- 不把 codebase indexing 伪装成 memory。代码、docs、git/PR 历史应走 index/search substrate。
- 不让 review 成为唯一保存路径。review 是纠偏和治理层，自动候选/自动高置信 promotion 仍然需要存在。

## 4. Breaking Compatibility Policy

### 4.1 Schema policy

v2 使用新的 schema family。启动时如果检测到旧 schema：

```text
remem detects legacy schema
-> refuse to start writable commands
-> print explicit reset/import instructions
```

允许的路径：

```bash
remem admin backup --output ~/.remem/backups/remem-v1-YYYYMMDD.sqlite
remem admin reset-v2 --confirm-destructive
remem import legacy --source ~/.remem/backups/remem-v1-YYYYMMDD.sqlite --best-effort
```

`import legacy` 是离线导入工具，不是 runtime compatibility。导入失败不能影响 v2 worker/capture 运行。

### 4.2 Host policy

v2 不允许 `host = unknown`。

Hook 不能识别 host 时：

- 不写 task。
- 写一条 structured error log。
- `doctor` 报错，提示重新 `remem install --target ...`。

合法 host 起步为：

```text
codex-cli
claude-code
```

未来可扩展：

```text
cursor
augment
manual
```

### 4.3 Hook install policy

v2 install 重新生成 hook config，不尝试 merge 旧 remem hook blocks。

安全规则：

- 保留非 remem hooks。
- 删除旧 remem-managed blocks。
- 重新写入 v2 blocks。
- 写入前备份原配置。

## 5. System Architecture

```text
Host hook
  -> capture_event / capture_message
  -> captured_events + event_blobs
  -> task coalescer
  -> extraction_tasks

Worker daemon
  -> scheduler tick
  -> claim task by identity + lease
  -> extract summaries / observations / candidates
  -> persist derived knowledge
  -> update task cursor/progress
  -> enqueue follow-up if budget exhausted

Promotion engine
  -> memory_candidates
  -> auto-promote high-confidence low-risk
  -> review queue for high-impact or ambiguous candidates
  -> curated memories + memory_events

Context compiler
  -> rules
  -> active workstream/session summary
  -> relevant curated memories
  -> raw fallback only when explicit or curated insufficient
  -> code/docs index through dedicated retrieval
```

## 6. Core Data Model

Names below are logical table names. Implementation may split modules, but semantics must remain.

### 6.1 `hosts`

```sql
CREATE TABLE hosts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,          -- codex-cli, claude-code
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at_epoch INTEGER NOT NULL
);
```

No `unknown` host.

### 6.2 `workspaces`

```sql
CREATE TABLE workspaces (
    id INTEGER PRIMARY KEY,
    root_path TEXT NOT NULL UNIQUE,
    git_remote TEXT,
    git_branch TEXT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);
```

`root_path` is the stable workspace boundary used by install/config detection.

### 6.3 `projects`

```sql
CREATE TABLE projects (
    id INTEGER PRIMARY KEY,
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_path TEXT NOT NULL,
    project_key TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(workspace_id, project_path)
);
```

`project_key` is a normalized identity used in MCP responses and context filtering.

### 6.4 `sessions`

```sql
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_id TEXT NOT NULL,
    started_at_epoch INTEGER,
    last_seen_at_epoch INTEGER NOT NULL,
    status TEXT NOT NULL,               -- active, stopped, abandoned
    UNIQUE(host_id, project_id, session_id)
);
```

### 6.5 `captured_events`

Append-only event ledger.

```sql
CREATE TABLE captured_events (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER NOT NULL REFERENCES sessions(id),
    session_id TEXT NOT NULL,
    turn_id TEXT,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,           -- user_message, assistant_message, tool_call, tool_result, file_edit, session_stop
    role TEXT,                          -- user, assistant, tool, system
    tool_name TEXT,
    content_text TEXT,
    content_blob_id INTEGER,
    content_hash TEXT NOT NULL,
    token_estimate INTEGER NOT NULL DEFAULT 0,
    retention_class TEXT NOT NULL,      -- raw_keep, raw_compact, raw_drop_candidate
    created_at_epoch INTEGER NOT NULL,
    inserted_at_epoch INTEGER NOT NULL,
    UNIQUE(host_id, session_id, event_id)
);
```

Rules:

- Small content goes into `content_text`.
- Large content goes into `event_blobs`.
- Very large low-value tool output is compacted before storage.
- Nothing in this table is automatically treated as durable memory.

### 6.6 `event_blobs`

```sql
CREATE TABLE event_blobs (
    id INTEGER PRIMARY KEY,
    content_hash TEXT NOT NULL UNIQUE,
    content_encoding TEXT NOT NULL,     -- plain, gzip
    content_bytes BLOB NOT NULL,
    original_bytes INTEGER NOT NULL,
    stored_bytes INTEGER NOT NULL,
    created_at_epoch INTEGER NOT NULL
);
```

Hard limits:

- Single event raw text max before compaction: 256 KiB.
- Single stored blob max after compression: 512 KiB.
- Larger content is stored as a digest + prefix/suffix summary, not full raw text.

### 6.7 `extraction_tasks`

Unified reliable queue.

```sql
CREATE TABLE extraction_tasks (
    id INTEGER PRIMARY KEY,
    task_kind TEXT NOT NULL,            -- session_rollup, observation_extract, memory_candidate, rule_candidate, index_update
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER,
    priority INTEGER NOT NULL,
    status TEXT NOT NULL,               -- pending, processing, delayed, done, failed
    idempotency_key TEXT NOT NULL UNIQUE,
    cursor_event_id INTEGER,
    high_watermark_event_id INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_retry_epoch INTEGER,
    lease_owner TEXT,
    lease_expires_epoch INTEGER,
    last_error TEXT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);
```

Task coalescing:

- Multiple hook events for the same identity update `high_watermark_event_id`.
- They do not create unbounded duplicate jobs.
- A task's progress is measured by event ids advanced, not by number of observations produced.

### 6.8 `session_summaries`

```sql
CREATE TABLE session_summaries (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER NOT NULL REFERENCES sessions(id),
    summary_text TEXT NOT NULL,
    covered_from_event_id INTEGER NOT NULL,
    covered_to_event_id INTEGER NOT NULL,
    model TEXT,
    created_at_epoch INTEGER NOT NULL,
    UNIQUE(session_row_id, covered_from_event_id, covered_to_event_id)
);
```

### 6.9 `observations`

Observations are extracted facts, not necessarily long-term memories.

```sql
CREATE TABLE observations (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER NOT NULL REFERENCES sessions(id),
    observation_type TEXT NOT NULL,      -- action, discovery, error, decision_hint, preference_hint
    text TEXT NOT NULL,
    evidence_event_ids TEXT NOT NULL,   -- JSON array of captured_events ids
    confidence REAL NOT NULL,
    created_at_epoch INTEGER NOT NULL
);
```

### 6.10 `memory_candidates`

```sql
CREATE TABLE memory_candidates (
    id INTEGER PRIMARY KEY,
    project_id INTEGER,
    scope TEXT NOT NULL,                -- global, workspace, project
    memory_type TEXT NOT NULL,          -- decision, discovery, bugfix, architecture, preference
    topic_key TEXT NOT NULL,
    text TEXT NOT NULL,
    evidence_event_ids TEXT NOT NULL,
    confidence REAL NOT NULL,
    risk_class TEXT NOT NULL,           -- low, medium, high
    review_status TEXT NOT NULL,        -- auto_promoted, pending_review, approved, edited, discarded
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);
```

### 6.11 `memories`

```sql
CREATE TABLE memories (
    id INTEGER PRIMARY KEY,
    project_id INTEGER,
    scope TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    topic_key TEXT NOT NULL,
    text TEXT NOT NULL,
    evidence_event_ids TEXT NOT NULL,
    source_candidate_id INTEGER,
    confidence REAL NOT NULL,
    status TEXT NOT NULL,               -- active, stale, superseded, rejected
    stale_after_epoch INTEGER,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(scope, COALESCE(project_id, 0), topic_key)
);
```

FTS/vector/multihop indexes attach to `memories`, not raw event tables.

### 6.12 `rule_candidates`

Stable operating rules should become AGENTS/rules candidates, not hidden memories.

```sql
CREATE TABLE rule_candidates (
    id INTEGER PRIMARY KEY,
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER,
    rule_path TEXT,
    rule_text TEXT NOT NULL,
    evidence_event_ids TEXT NOT NULL,
    confidence REAL NOT NULL,
    review_status TEXT NOT NULL,        -- pending_review, approved, discarded, exported
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);
```

remem may propose rule edits, but must not silently edit `AGENTS.md`, `CLAUDE.md`, or hook configs.

### 6.13 `worker_heartbeats`

```sql
CREATE TABLE worker_heartbeats (
    owner TEXT PRIMARY KEY,
    pid INTEGER,
    mode TEXT NOT NULL,                 -- daemon, once
    started_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);
```

## 7. Capture Path

### 7.1 Hook contract

Hook responsibilities:

1. Detect `host`.
2. Resolve workspace/project/session identity.
3. Normalize event payload.
4. Insert `captured_events`.
5. Coalesce/update `extraction_tasks`.
6. Return quickly.

Hooks must not:

- Call AI.
- Drain tasks.
- Run broad filesystem scans.
- Mutate rules/memories directly.
- Silently drop events when identity is invalid.

### 7.2 Event normalization

Every adapter maps native events into canonical event types:

```text
user_message
assistant_message
tool_call
tool_result
file_edit
session_stop
```

Codex initial support:

- `PostToolUse(Bash)` -> `tool_result`
- `Stop` -> `session_stop`
- transcript messages when path is available -> `user_message` / `assistant_message`

Claude Code initial support:

- `UserPromptSubmit` -> `user_message`
- `PostToolUse(Write/Edit/NotebookEdit/Bash/Task)` -> `tool_result` or `file_edit`
- `Stop` -> `session_stop`

### 7.3 Capture filtering

Filtering is allowed only at the retention layer, not by pretending the event never happened.

Examples:

- Short successful `pwd`, `ls`, `git status` outputs: keep compact metadata, not full output.
- Huge build logs: keep command, exit code, digest, first/last N lines, error spans.
- Secrets: redact known token patterns before storage.

## 8. Task Scheduling and Worker Semantics

### 8.1 Task kinds

```text
session_rollup
observation_extract
memory_candidate
rule_candidate
index_update
```

Priority order:

1. `session_rollup`: keeps recent context useful.
2. `memory_candidate`: promotes durable high-value facts.
3. `observation_extract`: lower-level facts for search/timeline.
4. `rule_candidate`: stable rule suggestions.
5. `index_update`: non-urgent local index maintenance.

### 8.2 Progress invariant

The most important invariant:

> Worker progress is counted by claimed/advanced event ranges, not by generated memory/observation count.

This avoids the liveness bug where a batch produces zero observations and the worker falsely treats the queue as drained.

### 8.3 Lease and retry

All task claims are lease-based:

```text
pending -> processing with lease_owner + lease_expires_epoch
processing -> done only after derived writes commit
processing -> delayed on transient failure
processing -> failed on permanent failure after max attempts
expired processing -> pending by scheduler recovery
```

Retry policy:

- AI rate limit: exponential backoff with jitter.
- Parse failure: retry once with repair prompt, then failed.
- Empty extraction: mark event range advanced, not failed.
- Invalid identity: failed and visible in doctor.

### 8.4 Worker modes

```bash
remem worker --once
remem worker --daemon
```

`--once`:

- Recover expired leases.
- Process ready tasks until budget exhausted or queue empty.
- Exit.

`--daemon`:

- Heartbeat.
- Recover leases.
- Schedule stale work.
- Process tasks.
- Sleep with backoff when idle.

Stop hook behavior:

- Always enqueue/coalesce tasks.
- If daemon heartbeat is healthy, return.
- If daemon is absent/unhealthy, spawn `worker --once`.

## 9. Backpressure and Budgeting

### 9.1 Capture budgets

Hard defaults:

```text
max_event_text_before_compact = 256 KiB
max_blob_after_compress = 512 KiB
max_events_per_session_per_hour = 2000
max_low_value_tool_events_per_session_per_hour = 300
```

When over budget:

- Chat messages remain highest priority.
- File edits and failed commands remain high priority.
- Repetitive successful Bash output is compacted.
- Low-value duplicates are recorded as aggregate counters.

### 9.2 AI budgets

Initial safe defaults:

```text
max_concurrent_ai_tasks_per_host = 1
max_ai_seconds_per_worker_once = 240
max_ai_tasks_per_daemon_tick = 4
max_events_per_extraction_batch = 30
```

These are operator-visible and should appear in `remem status --verbose`.

### 9.3 Queue budgets

`extraction_tasks` should not grow linearly with events. Coalescing by identity and kind is required.

Bad:

```text
one tool result -> one observation job
```

Good:

```text
many events in same session -> one task with moving high_watermark
```

## 10. Promotion and Review Policy

### 10.1 Candidate creation

Memory candidates are created from observations/session summaries when the model detects:

- Long-term user preference.
- Project architecture decision.
- Debugging root cause + fix.
- Important discovery with future implications.
- Stable workflow/process rule.

### 10.2 Auto-promotion

Auto-promote only when all are true:

- `confidence >= 0.82`
- `risk_class = low`
- Memory type is `discovery`, `bugfix`, or low-risk `preference`
- Evidence includes at least one user/assistant message or explicit command result.
- Topic key does not conflict with an active memory.

Require review when:

- Memory changes security, auth, payments, keys, deployment, or data deletion behavior.
- It proposes editing rules files.
- It conflicts with an existing active memory.
- It is global scope.
- It is inferred only from tool output without user/assistant confirmation.

### 10.3 Review UX

CLI:

```bash
remem review list
remem review show <candidate_id>
remem review approve <candidate_id>
remem review edit <candidate_id> --text ...
remem review discard <candidate_id>
```

MCP:

```text
list_memory_candidates
approve_memory_candidate
discard_memory_candidate
```

Review is not required for the system to function, but pending candidates must be visible.

## 11. Retrieval and Context Compilation

### 11.1 Retrieval substrates

Use the right substrate for the right question:

| Need | Source |
| --- | --- |
| Stable instructions | AGENTS.md / CLAUDE.md / rules |
| User/project long-term memory | `memories` |
| Recent work continuity | `session_summaries` |
| Debug timeline | `observations` + captured event provenance |
| Literal old phrase | raw `captured_events` / raw messages FTS |
| Codebase knowledge | code/docs/git index, not memory DB |

### 11.2 Default context compiler order

1. Active rules and project instructions.
2. Current workstream and latest session summary.
3. Relevant active memories.
4. Recent observations for the same project/session.
5. Raw fallback only when explicitly requested or curated results are insufficient.

### 11.3 Raw fallback

Raw fallback must be labeled:

```text
This came from raw archive, not curated memory.
```

This prevents raw noise from being treated as stable truth.

## 12. CLI and MCP Surface

### 12.1 CLI commands

```bash
remem status
remem doctor
remem worker --once
remem worker --daemon
remem review list
remem review approve <id>
remem review discard <id>
remem admin backup
remem admin reset-v2 --confirm-destructive
remem import legacy --source <db>
```

### 12.2 MCP tools

Keep core tools:

```text
search
search_raw
get_observations
save_memory
```

Add review tools:

```text
list_memory_candidates
approve_memory_candidate
discard_memory_candidate
```

`save_memory` remains a manual supplement. It must not be the primary memory path.

## 13. Doctor and Status

`remem status` must show:

```text
Capture:
  Events today
  Compacted events
  Dropped low-value aggregates

Tasks:
  Pending / processing / delayed / failed
  Oldest pending age
  Top identities by ready work

Worker:
  Daemon health
  Last heartbeat
  Lease recovery count

Knowledge:
  Session summaries
  Observations
  Memory candidates pending review
  Active memories

Budgets:
  AI tasks used
  Queue pressure
  Capture compaction rate
```

`remem doctor` fails when:

- Host identity cannot be detected.
- Hooks are missing for selected target.
- DB schema is legacy and writable commands are attempted.
- Daemon is configured but not healthy.
- Processing leases are expired beyond threshold.
- Failed task count exceeds threshold.
- Capture compaction/drop rate exceeds threshold for sustained periods.

## 14. Security and Data Integrity

- Use parameterized SQL for every query.
- Never execute shell commands from DB content.
- Never store secrets unredacted when pattern match detects them.
- Do not print secrets in logs, doctor, or status.
- Do not silently edit high-context files (`AGENTS.md`, `CLAUDE.md`, hooks).
- Every memory must retain evidence ids.
- Every destructive admin command must create a backup unless explicitly disabled with a second confirmation flag.

## 15. Implementation Plan

### Phase 0: Freeze and replace design target

Deliverables:

- Land this SPEC.
- Mark older drain spec as superseded in docs/README or SPEC index.
- Stop expanding old `pending_observations` compatibility work.

Validation:

```bash
cargo check
```

### Phase 1: New schema and typed identity

Files likely touched:

- `src/db.rs`
- `src/db_models.rs`
- `src/migrate/*`
- new `src/identity.rs`
- new migrations for v2 schema family

Deliverables:

- New tables from this SPEC.
- No `unknown` host.
- Writable command refuses legacy schema.
- Backup/reset admin commands.

Validation:

```bash
cargo test migration -- --nocapture
cargo test identity -- --nocapture
cargo check
```

### Phase 2: Capture ledger

Files likely touched:

- `src/observe/*`
- `src/summarize/summary_job/hook.rs`
- adapter modules
- new `src/capture/*`

Deliverables:

- Hooks write `captured_events`.
- Large payload compaction.
- Task coalescing by identity.
- No AI in hook path.

Validation:

```bash
cargo test capture -- --nocapture
cargo test observe -- --nocapture
cargo check
```

### Phase 3: Extraction task queue and worker

Files likely touched:

- replace or rewrite `src/db_job/*`
- replace or rewrite `src/db_pending/*`
- `src/worker.rs`
- new `src/extraction/*`

Deliverables:

- Lease-based task queue.
- Worker progress by event range.
- Daemon heartbeat.
- Stop fallback based on heartbeat.
- Retry/backoff.

Validation:

```bash
cargo test extraction_task_claims_are_lease_scoped -- --nocapture
cargo test worker_advances_empty_extraction_ranges -- --nocapture
cargo test daemon_heartbeat_controls_stop_fallback -- --nocapture
cargo check
```

### Phase 4: Derived knowledge and promotion

Files likely touched:

- `src/summarize/*`
- `src/memory_service/*`
- new `src/promotion/*`
- MCP handlers

Deliverables:

- Session summaries from event ranges.
- Observations with evidence ids.
- Memory candidates.
- Auto-promotion policy.
- Review commands and MCP tools.

Validation:

```bash
cargo test memory_candidate -- --nocapture
cargo test auto_promotion_policy -- --nocapture
cargo test review -- --nocapture
cargo check
```

### Phase 5: Retrieval and context compiler

Files likely touched:

- `src/search/*`
- `src/context/*`
- MCP server handlers

Deliverables:

- Curated-first search.
- Raw fallback labeled as raw.
- Context compiler budget tiers.
- Provenance returned with memory hits.

Validation:

```bash
cargo test search_curated_first -- --nocapture
cargo test raw_fallback_is_labeled -- --nocapture
cargo test context_budget -- --nocapture
cargo check
```

### Phase 6: Install, doctor, and operational rollout

Files likely touched:

- `src/install/*`
- `src/doctor/*`
- `src/cli/actions/query/status.rs`
- README docs

Deliverables:

- v2 hook install.
- Optional daemon install.
- Status/doctor for all core metrics.
- Clear legacy-schema refusal message.

Validation:

```bash
cargo test install -- --nocapture
cargo test doctor -- --nocapture
cargo test status -- --nocapture
cargo check
cargo test
```

## 16. Acceptance Criteria

Functional:

- Hook path remains AI-free and fast.
- Every accepted event lands in `captured_events` or an explicit aggregate/drop record.
- High-frequency Bash sessions do not create unbounded task rows.
- Worker daemon drains tasks without future Stop hooks.
- Expired leases recover automatically.
- Empty extraction output still advances task progress.
- Curated memories include evidence ids and confidence.
- Raw archive is searchable but clearly labeled as raw.

Quality:

- Long-term context contains decisions, preferences, bug fixes, architecture facts, and important discoveries.
- Low-value raw tool noise does not become durable memory.
- Conflicting memories are not silently overwritten.
- Stable operating rules become reviewable rule candidates.

Performance:

- AI concurrency is capped.
- Queue growth is coalesced by identity.
- Large payloads are compacted.
- Status exposes queue pressure before backlog becomes invisible.

Compatibility:

- No runtime compatibility with old queue/schema.
- Legacy DB is refused unless reset/import is explicitly requested.
- Codex and Claude Code are isolated by host identity.

Verification:

```bash
cargo check
cargo test
cargo build --release
```

Manual E2E:

1. Install v2 hooks for Codex.
2. Run a long Bash-heavy Codex session.
3. Confirm `captured_events` grows but `extraction_tasks` stays coalesced.
4. Confirm daemon drains tasks without another Stop hook.
5. Confirm one high-value decision becomes a memory candidate or active memory.
6. Confirm low-value Bash noise is not promoted.
7. Confirm `search_raw` can find literal raw content with raw label.
8. Confirm `search` returns curated memory first.

## 17. Open Decisions

Recommended defaults:

- Auto-promote threshold: `confidence >= 0.82`.
- AI concurrency: `1` per host.
- Extraction batch size: `30` events.
- Worker once budget: `240s`.
- Daemon install: opt-in first, default only after manual validation.
- Legacy import: best-effort offline only.

Open questions before implementation:

1. Should `captured_events.content_text` store assistant full text by default or compact it like tool output?
2. Should vector search attach to observations as well as memories?
3. Should global-scope memories always require review?
4. Should `rule_candidates` export patches or only plain text suggestions?
5. Should old raw transcripts be replayed into v2 during optional import?

## 18. Decision

Proceed with v2 as a breaking architecture change.

The old design can be patched, but patching it preserves the wrong abstraction: a raw pending queue pretending to be a memory system. v2 keeps remem's correct product bet, automatic capture, while moving durable memory quality into the right layers: staged evidence, reliable extraction, governed promotion, and budgeted context compilation.
