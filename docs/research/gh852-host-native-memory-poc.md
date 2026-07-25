# GH-852 Host-Native Memory — PoC Evidence and Hooks Coverage Audit

Refs #852 / Refs #849. Companion to `specs/GH852/{product,tech,tasks}.md`.

Authorization context: the SP852-T4 human gate was recorded by the maintainer
on issue #852 (2026-07-24 comment, blanket delegation). Evidence below is
split into **observed facts** (real host or code citations) and **runtime
evidence still pending**. Nothing pending is presented as observed.

## 1. Codex native memories — structural PoC (SP852-T2, partial, real host)

Observed facts (read-only inspection of a real user installation on this
machine, 2026-07-24; directory structure only, no memory content copied or
committed):

- Host: `codex-cli 0.145.0` (`codex --version`).
- `~/.codex/memories/` exists and contains aggregate files (`MEMORY.md`,
  `raw_memories.md`, `memory_summary.md`), a `rollout_summaries/` directory,
  and several non-memory entries (`.git/`, `.tmp*/`, `extensions/`, `skills/`,
  `git_tmp/`, `xcrun_db`). The memories root is therefore **not** a safe
  import surface; per-record import targets `rollout_summaries/` only.
- `rollout_summaries/` contained 95 entries; **95/95** were regular `.md`
  files matching the filename pattern
  `YYYY-MM-DDTHH-MM-SS-<4 alnum>-<slug>.md`; zero unknown entries, zero
  symlinks, zero subdirectories.
- **95/95** files begin with a `key: value` header terminated by a blank
  line. Observed header key orders (exactly two):
  `thread_id, updated_at, rollout_path, cwd` and
  `thread_id, updated_at, rollout_path, cwd, git_branch`.
- `updated_at` was RFC3339 with UTC offset (`NNNN-NN-NNTNN:NN:NN+NN:NN`) in
  all files; `rollout_path` and `cwd` were absolute paths in all files.
- A markdown body follows the header (first line `# …`).

This freezes the detector's closed format set as `codex-rollout-summary/v1`
(`src/cli/actions/codex_memory_import/parser.rs`). Unknown files, unknown
header keys, non-RFC3339 timestamps, relative paths, or empty bodies fail the
whole batch (B-006).

Design inference (not observed): the aggregate files appear to be derived from
the rollout summaries (`raw_memories.md` embeds `rollout_summary_file:`
references). They are excluded from import; importing them would duplicate the
per-record source.

Runtime evidence still pending for SP852-T2 (isolated-HOME PoC): behavior of
other codex-cli versions, concurrent-write behavior while Codex is running,
and generation timing. The shipped importer fails visibly on any fingerprint
drift, so unsupported versions cannot be silently imported.

## 2. Claude `autoMemoryDirectory` (SP852-T1) — not executed; bridge is no-go

No isolated real-host Claude PoC was executed in this implementation window.
Per tech spec §2 and B-016/B-019, absent PoC proof the native delivery bridge
is **no-go on every host/version** and remem stays `hook_only`:

- No code path writes `autoMemoryDirectory`, takes over a directory, or emits
  a `MEMORY.md` delivery block. Default behavior toward user `~/.claude`
  surfaces is unchanged.
- `src/context/claude_memory/ownership.rs` reports the bridge state
  (`hook_only`, with the no-go reason) and surfaces any user-owned
  `autoMemoryDirectory` value read-only; `remem doctor` shows it.
- Closure audit of the native input path landed (see §3), which is a
  precondition for any future activation.

Runtime evidence still pending for SP852-T1: effective-settings resolution
across scopes, actual startup-load window and capacity of `MEMORY.md`, hook
failure propagation, lifecycle/rollback behavior. Until a human records a
per-host go decision at that evidence, activation code must not ship.

## 3. Claude native-memory input closure audit (B-019) — implemented

Previous state on main (`src/observe/native.rs` before this change):
Write/Edit events on `.claude/projects/*/memory/*.md` (except `MEMORY.md`)
were inserted **directly into active memories** via
`insert_memory_with_branch`, failures were logged as `warning` and dropped,
and remem's own `remem_sessions.md` matched the ingest filter (self-ingest).

Shipped fixes:

- `remem_sessions.md` (remem-owned delivery file) is excluded from ingestion.
- Topic files now land as `memory_candidates` rows with
  `source_kind=claude_native`, `source_trust_class=external_content`,
  `review_status=pending_review` (or `quarantined` on instruction-pattern
  match), never directly in active memories.
- Ingestion failures log at error level and propagate to the hook exit status
  (`src/observe/hook.rs`), replacing warning-only degradation.

## 4. Codex `hooks.json` coverage audit (SP852-T3) — static portion

Baseline (code citations, this revision):

- Core installer `remem install --target codex` sets
  `[features].hooks = true` in `~/.codex/config.toml` and merges remem hooks
  into `~/.codex/hooks.json` (`src/install/hosts/codex.rs::install_hooks`,
  `apply_codex_hooks_json`).
- The installed Codex event set from `build_hooks(bin, HookStrategy::Codex)`
  (`src/install/config.rs`) is exactly: `SessionStart` → `remem context
  --host codex-cli` (timeout 15000 ms) and `Stop` → `remem summarize --host
  codex-cli` (timeout 120000 ms). Timeouts are converted to Codex units by
  `convert_hook_timeouts_to_seconds`.
- Claude-only events (`HookStrategy::ClaudeCode` branch): `UserPromptSubmit`
  (session-init), `PreToolUse` matcher `Bash` (rules eval), `PostToolUse`
  matcher `Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task` (observe),
  `PreCompact` (summarize). None of these are installed for Codex.
- Plugin activation is delegation, not a second implementation:
  `plugins/remem/scripts/activate-codex.js` runs
  `remem install --target codex --hooks-only`, so the effective `hooks.json`
  baseline is produced by the same core code path. Plugin-only MCP loading
  installs no hooks (`plugins/remem/README.md`).

Coverage gap vs Claude (static conclusion): Codex capture has **no observe
level tool events** (no PreToolUse/PostToolUse equivalents installed), so
per-tool-call evidence, git evidence, and native-memory file-write ingestion
have no Codex-side trigger; capture density relies on SessionStart/Stop.

Go/no-go: **no-go for changing the capture chain in this issue** (by spec:
audit only). The remaining go/no-go question — whether official Codex hook
events could deliver observe-grade input fields with acceptable failure
semantics — requires isolated real-session event matrices for all three
states (core install, plugin hooks-only activation, plugin-only MCP control).
That runtime evidence is pending; no conclusion is fabricated here. Any
future hooks change needs a separate issue/spec with its own human gates.

## 5. Cleanup statement

The structural PoC in §1 performed read-only `ls`/`stat`/header-shape checks;
no file under `~/.codex/` or `~/.claude/` was created, modified, or deleted,
and no memory content left the machine. All import/doctor tests use synthetic
fixtures in temp directories with `ScopedTestDataDir` isolation.
