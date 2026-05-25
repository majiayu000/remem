# Spec: Codex Context Injection Gating

**Status**: Draft
**Date**: 2026-05-25
**Related**: `docs/spec-context-compiler.md`, `docs/sessionstart-context-smoke.md`

## 1. Background

Codex runs `SessionStart` hooks at the start of an agent task/turn. The hook output is recorded as developer context in the conversation history, and later model requests are built from the accumulated session history.

For remem, the installed Codex hook currently runs:

```text
REMEM_CONTEXT_HOST=codex-cli remem context
```

`remem context` renders a full startup context block every time it is invoked. In a single visible Codex thread, multiple user requests can create multiple tasks/turns. Each task can therefore append another full remem context block to the same conversation history.

This is correct from Codex's hook lifecycle, but it is not the desired remem behavior.

## 2. Problem

Repeated full context injection causes:

- prompt noise: the same preferences, core memories, workstreams, and sessions appear multiple times;
- token waste: each injected block remains eligible for future prompts until compaction or truncation;
- accidental re-weighting: repeated memories may look more important to the model than they are;
- poor UX: users see memory text "inserted in the middle" of a long chat;
- stale context risk: an old injected block can stay in history after a newer block is injected.

The core issue is not retrieval quality. It is lifecycle mismatch:

```text
remem assumes SessionStart means one visible conversation start
Codex uses SessionStart for task/turn startup inside a persistent thread
```

## 3. Goals

- Prevent duplicate full `remem context` blocks in the same Codex thread.
- Preserve high-quality first-turn context for new Codex sessions.
- Allow updated context to appear only when it materially changes.
- Keep the behavior deterministic and testable without vectors or external services.
- Keep failures safe: if gating state cannot be read or written, output the context and log the reason.
- Provide debug visibility so users can prove why context was emitted or suppressed.
- Keep Claude Code behavior unchanged unless explicitly enabled later.

## 4. Non-Goals

- No vector/reranker/hybrid retrieval change.
- No change to MCP tool contracts.
- No deletion or mutation of memories as part of injection gating.
- No Codex source patch in this work.
- No attempt to suppress Codex's own conversation history.
- No silent loss of first-turn context.
- No global one-context-per-project rule; independent Codex sessions must still get startup context.

## 5. Design Principle

SessionStart output should be **idempotent per Codex conversation**, not per process invocation.

The renderer still builds a high-quality context block. A new gate decides whether that block should be printed to stdout:

```text
hook stdin / CLI args
  -> ContextInvocation
  -> render candidate context
  -> fingerprint candidate context
  -> ContextInjectionGate
  -> emit full context | emit compact update | emit nothing
```

No stdout means no developer context is injected by Codex. Diagnostics must go to remem logs or debug-only stderr, not stdout.

## 6. Current Code Anchors

| Area | Current behavior |
| --- | --- |
| `src/install/config.rs` | Installs Codex `SessionStart` as `REMEM_CONTEXT_HOST=codex-cli remem context`. |
| `src/cli/types.rs` | `context` already accepts `--session_id`, `--host`, `--debug`, but install does not pass session data. |
| `src/cli/dispatch.rs` | `Commands::Context` calls `context::generate_context(...)` directly and does not parse hook stdin. |
| `src/context/render.rs` | Opens DB, loads context data, renders all sections, prints to stdout. |
| `src/context/host.rs` | Codex profile is host-aware and already records that Codex is Stop/context-focused. |
| `docs/spec-context-compiler.md` | Defines the desired host-aware context compiler direction. |

## 7. Proposed Behavior

### 7.1 Default Codex Policy

For `REMEM_CONTEXT_HOST=codex-cli`:

1. First invocation for a Codex session emits full context.
2. Later invocations for the same session emit nothing if the rendered context fingerprint is unchanged.
3. Later invocations may emit a compact update only when the fingerprint changes.
4. If the session cannot be identified, use a short fallback cooldown keyed by host + project + cwd + transcript path when available.
5. If gating fails due to DB or state errors, emit full context and log `gate=fail_open`.

For `claude-code` and `unknown`:

- preserve current behavior in the first implementation slice;
- allow future opt-in via env var after Codex behavior is stable.

### 7.2 Full Context

The full context is the existing rendered output:

```text
# [project @branch] context ...
Use `search`/`get_observations` ...
...
context memories loaded ...
```

It is emitted when:

- no prior successful full context exists for the same injection key;
- the caller uses `REMEM_CONTEXT_GATE=off`;
- debug smoke tests explicitly request `--debug --force`;
- gating state cannot be trusted and fail-open applies.

### 7.3 Suppressed Context

When context is unchanged for the same Codex session, stdout must be empty.

Log line:

```text
[context-gate] suppress host=codex-cli key=... reason=same_hash age=...s hash=...
```

The log is enough for diagnosis. Printing a "context skipped" message to stdout would still be injected into Codex history, so it is not allowed by default.

### 7.4 Compact Update

When the rendered context fingerprint changed within the same session, default behavior should still be conservative:

- emit a compact update, not the full block;
- include only changed section names, counts, and retrieval instruction;
- stay under `REMEM_CONTEXT_DELTA_CHAR_LIMIT` default `1200`.

Example:

```text
# [project @branch] remem context update
Memory context changed since the last injection. Use `search`/`get_observations` for details.

Changed sections: preferences, workstreams
Stats: 16 memories, 6 core, 10 indexed, 11 preferences, 5 sessions.
```

Rationale: a long-running Codex thread already has prior full context in history. The update should inform the model that retrievable context changed without duplicating the whole memory map.

## 8. Identity Model

### 8.1 Hook Input Parsing

Add a lightweight hook envelope parser for `remem context`.

Codex `SessionStart` hook input can include:

```json
{
  "session_id": "...",
  "cwd": "...",
  "transcript_path": "...",
  "model": "...",
  "target": {
    "type": "SessionStart",
    "source": "Startup"
  }
}
```

The parser must:

- read stdin with a short timeout, reusing the existing `read_stdin_with_timeout` pattern;
- accept empty stdin for manual CLI use;
- prefer CLI args over stdin values when both exist;
- never log the full raw stdin payload unless debug is enabled and truncated.

Proposed type:

```rust
pub(super) struct ContextInvocation {
    pub cwd: String,
    pub project: String,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub host: HostKind,
    pub source: Option<String>,
    pub model: Option<String>,
    pub force: bool,
    pub debug: bool,
}
```

### 8.2 Injection Key

Primary key:

```text
host + session_id + project
```

Fallback key when `session_id` is missing:

```text
host + project + normalized_cwd + transcript_path_hash
```

Last fallback:

```text
host + project + normalized_cwd
```

The last fallback must use a short cooldown because it can group unrelated manual invocations.

## 9. State Storage

Add a small runtime table to the main remem DB:

```sql
CREATE TABLE IF NOT EXISTS context_injections (
    id INTEGER PRIMARY KEY,
    host TEXT NOT NULL,
    project TEXT NOT NULL,
    injection_key TEXT NOT NULL,
    session_id TEXT,
    transcript_path TEXT,
    context_hash TEXT NOT NULL,
    output_mode TEXT NOT NULL,
    output_chars INTEGER NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    last_emitted_epoch INTEGER NOT NULL,
    emit_count INTEGER NOT NULL DEFAULT 1,
    suppress_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE(host, injection_key)
);

CREATE INDEX IF NOT EXISTS idx_context_injections_project_seen
    ON context_injections(project, updated_at_epoch DESC);
```

`output_mode` values:

- `full`
- `delta`
- `suppressed`
- `fail_open`

State retention:

- keep at least 30 days;
- delete rows older than `REMEM_CONTEXT_GATE_RETENTION_DAYS`, default `30`;
- cleanup can run opportunistically during context gate evaluation.

## 10. Fingerprint

Hash the rendered candidate context after:

- section rendering;
- total char limit enforcement;
- stats footer generation.

Do not include:

- wall-clock timestamp in the header;
- debug trace;
- ANSI colors.

If the existing header contains a timestamp, the fingerprint function must normalize it before hashing. Otherwise unchanged memories would look changed every invocation.

Recommended fingerprint input:

```text
host
project
branch
retrieval_hint
preferences section body
core section body
index section body
workstreams section body
sessions section body
stats excluding total chars if affected only by timestamp
```

Use SHA-256 rather than `DefaultHasher`, because this is persisted across process runs.

## 11. Configuration

| Env var | Default | Meaning |
| --- | --- | --- |
| `REMEM_CONTEXT_GATE` | `auto` | `auto`, `off`, `strict`, `delta` |
| `REMEM_CONTEXT_GATE_HOSTS` | `codex-cli` | Comma-separated hosts where gating applies. |
| `REMEM_CONTEXT_GATE_FALLBACK_COOLDOWN_SECS` | `900` | Cooldown when no session id exists. |
| `REMEM_CONTEXT_DELTA_CHAR_LIMIT` | `1200` | Max chars for compact update output. |
| `REMEM_CONTEXT_GATE_RETENTION_DAYS` | `30` | State retention. |
| `REMEM_CONTEXT_FORCE` | `0` | Emit full context and update gate state. |
| `REMEM_CONTEXT_GATE_DEBUG` | `0` | Log gate decision details. |

Mode semantics:

- `auto`: full once per session, suppress unchanged repeats, delta on changed hash.
- `off`: always emit full context; matches current behavior.
- `strict`: full once per session, suppress all later repeats even if hash changes.
- `delta`: same as auto, but always prefer delta over full after first emission.

CLI additions:

```text
remem context --force
remem context --gate off|auto|strict|delta
```

The CLI flags override env vars.

## 12. Algorithm

```text
run context command
  parse CLI args
  parse hook stdin with timeout
  resolve ContextInvocation
  build ContextRequest
  render candidate context into String, not stdout
  if host not gated:
      print candidate
      return
  compute normalized context hash
  load context_injections row by host + injection_key
  if force or no row:
      upsert row output_mode=full, hash=current_hash, emit_count += 1
      print full candidate
      return
  if row.context_hash == current_hash:
      update suppress_count += 1, updated_at_epoch=now, output_mode=suppressed
      print nothing
      return
  if gate mode strict:
      update suppress_count += 1, context_hash=current_hash, output_mode=suppressed
      print nothing
      return
  build compact delta
  update row context_hash=current_hash, output_mode=delta, emit_count += 1
  print compact delta
```

Important: "print nothing" means no stdout at all, including trailing newline.

## 13. Failure Policy

This feature must be fail-open for first-turn context quality.

| Failure | Behavior |
| --- | --- |
| Cannot read stdin | Continue with CLI/env/cwd values. |
| Cannot parse stdin | Continue, log truncated parse error. |
| Cannot open DB | Existing `render_empty_state` behavior can stay for no-data cases; gate logs `fail_open_no_db`. |
| Cannot read gate table | Print full candidate, log `fail_open_gate_read`. |
| Cannot write gate row | Print full candidate, log `fail_open_gate_write`. |
| Cannot hash context | Print full candidate, log `fail_open_hash`. |

Do not silently suppress context on errors.

## 14. Implementation Plan

### Step 1: Render-to-buffer boundary

Refactor `src/context/render.rs`:

- keep `generate_context(...) -> Result<()>` as CLI boundary;
- add `render_context(request: &ContextRequest) -> Result<RenderedContext>`;
- move `print!("{}", output)` to the outer boundary;
- include stats and section metadata in `RenderedContext`.

Suggested type:

```rust
pub(super) struct RenderedContext {
    pub output: String,
    pub stats: ContextRenderStats,
    pub section_hash_inputs: Vec<SectionHashInput>,
}
```

### Step 2: Context hook input parser

Add `src/context/invocation.rs`:

- parse CLI args + optional stdin;
- reuse or extract `read_stdin_with_timeout`;
- produce `ContextInvocation`;
- unit-test empty stdin, Codex stdin, CLI override, malformed JSON.

### Step 3: Gate state module

Add `src/context/gate.rs`:

- `ContextGateMode`;
- `ContextInjectionKey`;
- `ContextFingerprint`;
- `ContextGateDecision`;
- `evaluate_context_gate(conn, invocation, rendered)`.

Keep SQL isolated in this module.

### Step 4: Migration

Add migration:

```text
v016_context_injection_gate.sql
```

Update:

- `src/migrate/types.rs`
- `src/migrate/tests.rs`
- any schema status checks if needed.

### Step 5: CLI and install updates

Update:

- `src/cli/types.rs`: add `--force`, `--gate`.
- `src/cli/dispatch.rs`: route through invocation parser and gate.
- `src/install/config.rs`: keep command simple if stdin parsing works.
- `src/install/tests.rs`: assert Codex hook command remains host-tagged.

Do not add shell redirection in the installed hook. Suppression belongs in remem, not in the hook command.

### Step 6: Smoke docs

Update `docs/sessionstart-context-smoke.md` with:

```bash
printf '{"session_id":"sess-1","cwd":"%s","transcript_path":"/tmp/codex.jsonl"}' "$PWD" \
  | REMEM_CONTEXT_HOST=codex-cli cargo run --quiet -- context

printf '{"session_id":"sess-1","cwd":"%s","transcript_path":"/tmp/codex.jsonl"}' "$PWD" \
  | REMEM_CONTEXT_HOST=codex-cli cargo run --quiet -- context | wc -c
```

Expected:

- first command emits full context;
- second command emits `0` bytes when context hash is unchanged.

## 15. Tests

### Unit Tests

Context invocation:

- parses Codex `SessionStart` JSON stdin;
- empty stdin falls back to cwd;
- CLI `--cwd` and `--session_id` override stdin;
- malformed stdin logs but does not fail the command.

Fingerprint:

- same context with different header timestamps has the same hash;
- changed preference/core/workstream/session content changes the hash;
- debug trace does not affect the hash.

Gate:

- first Codex invocation emits full context;
- second same-session same-hash invocation emits empty stdout;
- same session changed hash emits delta in `auto`;
- same session changed hash emits empty stdout in `strict`;
- `REMEM_CONTEXT_GATE=off` emits full every time;
- missing session id uses fallback cooldown;
- DB write failure fail-opens.

Migration:

- full migration on empty DB includes `context_injections`;
- migration SQL has no non-constant ALTER defaults;
- migration is idempotent.

### Integration / Smoke

Run:

```bash
cargo fmt --check
cargo check
cargo test context:: --lib
cargo test migrate:: --lib
cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tool/remem --host codex-cli --force
printf '{"session_id":"gate-smoke","cwd":"/Users/lifcc/Desktop/code/AI/tool/remem","transcript_path":"/tmp/remem-gate-smoke.jsonl"}' \
  | REMEM_CONTEXT_HOST=codex-cli cargo run --quiet -- context | wc -c
printf '{"session_id":"gate-smoke","cwd":"/Users/lifcc/Desktop/code/AI/tool/remem","transcript_path":"/tmp/remem-gate-smoke.jsonl"}' \
  | REMEM_CONTEXT_HOST=codex-cli cargo run --quiet -- context | wc -c
```

Expected:

- first `wc -c` is greater than `0`;
- second `wc -c` is `0` when no context changed.

Before submission:

```bash
cargo test
cargo clippy -- -D warnings
```

## 16. Observability

Add log lines with stable fields:

```text
[context-gate] emit host=codex-cli key=... mode=full hash=... chars=...
[context-gate] suppress host=codex-cli key=... reason=same_hash hash=...
[context-gate] emit host=codex-cli key=... mode=delta old_hash=... new_hash=... chars=...
[context-gate] fail_open host=codex-cli reason=gate_write error=...
```

Extend debug trace only when `REMEM_CONTEXT_DEBUG=1` or `--debug`:

```text
## Debug Trace
- gate mode=auto decision=suppressed key=... hash=...
```

Do not include gate debug trace in suppressed stdout. Suppressed means empty stdout.

## 17. Backward Compatibility

- Existing installed hooks keep working.
- Manual `remem context --cwd .` still prints full context unless host gating applies.
- `REMEM_CONTEXT_GATE=off` restores old behavior.
- Existing DBs migrate forward without data loss.
- No change to memory selection, scoring, or rendering quality in this spec.

## 18. Rollout

1. Land render-to-buffer and hook input parser behind no behavior change.
2. Land gate table and code with default `REMEM_CONTEXT_GATE=off` in tests only.
3. Enable default gating for `codex-cli`.
4. Run local Codex transcript smoke:
   - start a thread;
   - verify first task injects full context;
   - send a second user message in the same thread;
   - verify no duplicate remem developer block appears when hash is unchanged.
5. Update README / architecture docs after behavior is verified.

## 19. Risks

| Risk | Mitigation |
| --- | --- |
| First task loses context due to bad gate state | fail-open; `--force`; `REMEM_CONTEXT_GATE=off`. |
| Different Codex sessions collapse into one fallback key | prefer `session_id`; fallback cooldown only when session id missing. |
| Context changes are hidden too aggressively | default `auto` emits compact delta on hash change. |
| Timestamp makes every hash different | normalize timestamp before hashing. |
| Debug output gets injected | debug trace only when emitting; suppressed stdout stays empty. |
| State table grows forever | retention cleanup default 30 days. |

## 20. Open Questions

- Should `delta` include changed section item titles, or only section names and counts?
- Should `claude-code` opt into the same gate after Codex behavior proves stable?
- Should `remem doctor --target codex` report duplicate context injection count from `context_injections`?
- Should `remem status` expose recent gate suppressions and fail-open events?

## 21. Recommended First Slice

The first implementation slice should be deliberately small:

1. parse Codex hook stdin for `session_id`, `cwd`, and `transcript_path`;
2. render context to a buffer;
3. persist `context_injections`;
4. for `codex-cli`, emit full once per `session_id + project`, suppress identical repeats;
5. add `REMEM_CONTEXT_GATE=off` escape hatch;
6. add unit tests and the two-command smoke check.

Do not implement rich delta rendering in the first slice unless the suppress/full behavior is already stable.
