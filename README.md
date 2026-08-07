# remem: Local-first Coding Agent Memory for Claude Code and OpenAI Codex

[![MCP Toplist](https://mcptoplist.com/badge/io.github.majiayu000%2Fremem.svg)](https://mcptoplist.com/server/io.github.majiayu000%2Fremem)

> Stop re-explaining your project every new coding-agent session.

Language: **English** | [简体中文](README.zh-CN.md)

`remem` is a single Rust binary that automatically captures, distills, searches,
and injects project memory across Claude Code, OpenAI Codex, and Codex CLI
sessions. It keeps decisions, bug-fix rationale, project patterns, and
preferences available through hooks, MCP, CLI, and REST without requiring an
external database.

[![CI](https://github.com/majiayu000/remem/actions/workflows/ci.yml/badge.svg)](https://github.com/majiayu000/remem/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/majiayu000/remem?sort=semver)](https://github.com/majiayu000/remem/releases/latest)
[![crates.io](https://img.shields.io/crates/v/remem-ai)](https://crates.io/crates/remem-ai)
[![npm](https://img.shields.io/npm/v/%40remem-ai%2Fremem)](https://www.npmjs.com/package/@remem-ai/remem)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

![Remem recall demo — a new Claude Code session picks up last week's bug fix](assets/remem-recall-demo.gif)

*Real Claude Code session on a demo repo: a brand-new session recalls last week's root cause, commit, and open TODO — with memory citations, zero re-explaining.*

## What You Get

- Claude Code, OpenAI Codex, and Codex CLI remember project decisions across sessions.
- Cursor has a v1 integration: `remem install --target cursor` registers the
  remem MCP server (macOS/Linux), and the runtime commands for tool-event
  capture (`remem observe --host cursor`, GH-823) and Stop transcript
  summarization (`remem summarize --host cursor`, GH-825) are merged. The v1
  install surface does not register Cursor hook entries yet, so automatic
  Cursor memory capture is not enabled, and session-init injection is not
  supported on Cursor. See the [Cursor capability matrix](#cursor-capability-matrix).
- Bug-fix rationale, preferences, and project patterns are searchable.
- Memory stays local by default with SQLite and SQLCipher.
- Hooks, MCP tools, CLI commands, and a localhost REST API use the same store.
- Current-memory contracts expose staleness, temporal/as-of truth, citation
  usage, and injection audit state instead of treating recall as a black box.
- User-context controls keep personal claims, profile summaries, suppression
  feedback, and Markdown export explicit and reviewable.
- One Rust binary; no hosted database or separate memory service.

## Install

Install the `remem` binary:

```bash
brew install majiayu000/tap/remem
```

Then configure hooks and MCP for your installed coding agents:

```bash
REMEM_INSTALL_BINARY="$(brew --prefix remem)/bin/remem" remem install --target codex
# or: REMEM_INSTALL_BINARY="$(brew --prefix remem)/bin/remem" remem install --target claude
# or: REMEM_INSTALL_BINARY="$(brew --prefix remem)/bin/remem" remem install --target all
```

If you do not use Homebrew:

```bash
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | env REMEM_NO_CONFIG=1 sh
~/.local/bin/remem install --target codex
# or: ~/.local/bin/remem install --target claude
# or: ~/.local/bin/remem install --target all
```

`remem install` can auto-detect existing Claude Code, Codex CLI, and Cursor
config directories. On first-time setups, use `--target codex`,
`--target claude`, or `--target all` so remem can create the selected config
files.

`--target cursor` (macOS/Linux) manages only the user-level
`~/.cursor/hooks.json` and `~/.cursor/mcp.json`: it registers the remem MCP
server, strictly validates both files, and preserves foreign entries
semantically for the validated snapshot (coordinated updates use staged
apply with compensating rollback, not a cross-file atomic transaction, and
edits landing between the final comparison and the rename can still be
lost). Install contract v1 registers no Cursor hook entries — automatic
Cursor memory is not enabled — and `session-init` is not supported on Cursor.
`--target auto` includes Cursor only when a Cursor config is detected on
macOS/Linux; Windows is skipped with a diagnostic because no hook command
renderer is approved there, and `--repair` does not cover Cursor. Before
downgrading remem, run `remem uninstall --target cursor` with the current
version first.

### Cursor capability matrix

Capabilities below reflect the merged runtime, not planned work:

| Capability | Claude Code / Codex CLI | Cursor (v1) |
|---|---|---|
| MCP memory tools (search, `save_memory`, ...) | Registered by `remem install` | Registered by `remem install --target cursor` (macOS/Linux only) |
| Session-start memory injection | Installed hook | Not supported; doctor and install always print `session-init: not supported on cursor` |
| Automatic tool-event capture | Installed hook | Runtime command exists (`remem observe --host cursor`, strict fail-closed parsing of the verified generic tool events); install v1 registers no hook entry, so it is not automatic |
| Stop transcript summarization | Installed hook | Runtime command exists (`remem summarize --host cursor`, Stop-keyed transcript snapshot with explicit `degraded/<reason>` fallback); install v1 registers no hook entry, so it is not automatic |
| `remem doctor` | Hooks/MCP rows | Dedicated Cursor row reporting detected/configured/drift/collision plus fixed capability lines |
| `remem install --repair` | Claude hooks only | Not supported |
| Windows | Supported | Not supported (no approved hook command renderer) |

Run `remem doctor` when you want to verify or troubleshoot the integration.
If Claude Code reports a Hook Integrity Warning or doctor shows incomplete
Claude hooks, run:

```bash
remem install --target claude --repair
```

Repair mode restores only user-level Claude hooks in `~/.claude/settings.json`.
It preserves third-party hooks, does not write `.claude.json` MCP settings, and
does not initialize the runtime store or API token.

## Success Check

![remem install and SessionStart context injection](assets/remem-demo.gif)

*What `remem install` configures, and what a new Codex session receives at SessionStart. The demo uses a temp HOME and temp database; no private memories are shown.*

Start a new Claude Code or Codex CLI session after installation. remem should
inject relevant project memory at session start and summarize durable memory
after the session stops. Then run:

```bash
remem status
remem search "last decision"
```

For Codex CLI, `remem install` creates or updates:

- `~/.remem/.key` and the encrypted `~/.remem/remem.db`
- `~/.remem/config.toml` memory-AI profiles
- Codex MCP registration in `~/.codex/config.toml`
- Codex SessionStart/Stop hooks in `~/.codex/hooks.json`

For a Codex-only setup, `remem doctor` reports Schema, Key format, Database,
and the Codex Hooks/MCP rows as ok. If Claude Code config directories already
exist but were not auto-detected during installation, run
`remem install --target claude` or `remem install --target all`. If doctor warns
about multiple `remem` binaries, follow the printed install-path fix so hooks
keep using the intended binary.

## Install With Your Coding Agent

Paste this into Claude Code or Codex CLI:

> Install remem for this repository. Use the official README. Configure it for
> this agent, run `remem doctor`, verify that session memory is working, and
> summarize what was installed.

## Why remem if Claude Code and Codex already have memory?

Built-in memory is useful for concise preferences and stable project guidance.

remem is for engineering memory that needs to be searchable, auditable,
project-scoped, and recoverable:

- Search past decisions, bug fixes, and rationale with `remem search`
- Inspect why a memory was injected with `remem why`
- Keep memory local with SQLite and SQLCipher
- Use MCP and REST APIs from coding agents and local tools
- Track usage and background memory cost
- Avoid hand-maintaining large `MEMORY.md` or `CLAUDE.md` files

### How remem Compares in the Ecosystem

Snapshot from our
[memory-tool ecosystem survey (2026-03)](docs/research/claude-memory-mcp-ecosystem-2026-03.md);
check upstream projects for their current feature sets.

| | remem | Built-in memory files | claude-mem | mem0 / OpenMemory |
|---|---|---|---|---|
| Capture | Automatic hooks + LLM distillation | Manual editing | Automatic hooks | Agent calls save tools |
| Agents | Claude Code + Codex, one shared store | Per-tool files | Claude Code | Any MCP client |
| Storage | Local SQLite, optional SQLCipher encryption | Plain text files | SQLite + Chroma vector DB | Vector DB, hosted platform or local server |
| Retrieval | FTS + optional embeddings via CLI, MCP, REST | Loaded wholesale | Tiered vector search | Vector search |
| Runtime | Single Rust binary | None | Node worker + background service | Python service |
| Audit trail | `remem why`, provenance, usage and cost tracking | Git history | Not documented in survey | Not documented in survey |

## How remem Solves Session Amnesia

| Without remem | With remem |
|---|---|
| "We use FTS5 trigram tokenizer..." (every session) | Injected automatically from memory |
| "Do not use `expect()` in non-test code" (again) | Preference surfaced before you ask |
| "Last session we decided to..." (reconstruct manually) | Decision history with rationale |
| Bug context lost after session ends | Root cause + fix preserved |

## Other Install Channels

```bash
# Quick install options
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | env REMEM_NO_CONFIG=1 REMEM_VERSION=vX.Y.Z sh
~/.local/bin/remem install --target codex

curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | env REMEM_NO_CONFIG=1 sh
~/.local/bin/remem install --target codex

curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | env REMEM_NO_CONFIG=1 REMEM_INSTALL_DIR=/usr/local/bin sh
remem install --target codex

# npm wrapper
npm install -g @remem-ai/remem
remem install --target codex

# Cargo
cargo install remem-ai --bin remem
remem install --target codex

# Manual GitHub Release download
curl -LO https://github.com/majiayu000/remem/releases/latest/download/remem-darwin-arm64.tar.gz
tar xzf remem-darwin-arm64.tar.gz
mv remem ~/.local/bin/
codesign -s - -f ~/.local/bin/remem  # required on macOS ARM
~/.local/bin/remem install --target codex

# Build from source
git clone https://github.com/majiayu000/remem.git
cd remem
cargo build --release
cp target/release/remem ~/.local/bin/
codesign -s - -f ~/.local/bin/remem  # required on macOS ARM
~/.local/bin/remem install --target codex
```

Use one canonical `remem` command on PATH. Standalone and source installs
should normally live at `~/.local/bin/remem`; Windows standalone installs
should use `%USERPROFILE%\.local\bin\remem.exe`. If you install through a
package manager such as Homebrew or Cargo, update through that same channel
and avoid keeping a second manual copy earlier or later on PATH. `remem doctor`
and `remem install --dry-run` warn when multiple `remem` executables are
visible.

### Updating an Existing Install

When you replace the binary manually, rerun `remem install` so existing Claude Code
and Codex hook commands pick up the current host-aware settings:

```bash
cargo build --release
cp target/release/remem ~/.local/bin/
codesign -s - -f ~/.local/bin/remem  # required on macOS ARM
remem install --target all
```

Verify the installed hooks include host-specific context commands:

```bash
jq -r '.hooks.SessionStart[]?.hooks[]?.command' ~/.claude/settings.json
jq -r '.hooks.SessionStart[]?.hooks[]?.command' ~/.codex/hooks.json
```

Expected commands are host-only; model, executor, and context policy live in
`~/.remem/config.toml`:

```text
/Users/you/.local/bin/remem context --host claude-code
/Users/you/.local/bin/remem context --host codex-cli
```

## Use With Codex

`remem install --target codex` configures Codex in four ways:

- Enables Codex hooks with `[features].hooks = true` in `~/.codex/config.toml`
- Registers `remem` as an MCP server in `~/.codex/config.toml`
- Writes Codex hook commands to `~/.codex/hooks.json`
- Creates or updates `~/.remem/config.toml` memory-AI profiles

After restarting Codex, remem automatically injects relevant project memory at
session start and summarizes the session at stop. Codex can also call the MCP
tools exposed by `remem mcp`: current-state, curated/raw search, contextual
recall, timeline/detail/commit lookup, memory save/governance, reports, and
workstream tools. All 14 tools publish explicit side-effect annotations. The 13
JSON tools return both their legacy text content and matching structured
content with an output schema; `timeline_report` remains Markdown.

SessionStart keeps Core, Preferences, and Workstreams on their existing paths,
then applies one deterministic relevance budget across Lessons, the non-Core
MemoryIndex, and Sessions. The default keeps the single strongest matching
item (`REMEM_CONTEXT_RELEVANCE_K=1`); set the value to `0` to restore the legacy
per-section selection. The context footer and `remem status` expose the active
state, threshold, selected count, and closed drop-reason counts.

The default Codex integration is intentionally low-noise: it uses
`SessionStart` for context injection and `Stop` for background summarization.
For Codex hook invocations, remem emits the supported
`hookSpecificOutput.additionalContext` JSON shape so the memory block is
model-visible through the hook contract instead of asking the assistant to
repeat a separate `Remem context:` line. Codex may still show its own completed
hook context block in the UI; remem does not add a second assistant-rendered
status line.
Codex uses strict duplicate-injection gating via
`[memory_ai.hosts."codex-cli"].context_gate = "strict"`, so a mid-chat
`SessionStart` repeat stays silent after the first injection for the same
session. It does not install high-frequency Bash observation by default.

### Codex Plugin

This repository includes a local Codex plugin wrapper in `plugins/remem`.
The plugin exposes `remem mcp` and a Remem skill while keeping hook activation
explicit. The complete product direction is documented in
[`docs/spec-codex-plugin-complete-design.md`](docs/spec-codex-plugin-complete-design.md);
the current plugin is the local development foundation, not the final
self-contained plugin experience. To try it from a local checkout:

```bash
codex plugin marketplace add .
codex plugin add remem@remem-local
```

The plugin runtime gives `remem --version` up to 15 seconds on cold startup.
Set `REMEM_RUNTIME_VERSION_TIMEOUT_MS` to a different positive integer only
when the local filesystem or code-signature check needs another bound.

After installing the plugin, start a new Codex thread. To enable automatic
SessionStart context injection and Stop summarization, run:

```bash
cargo build --release
node plugins/remem/scripts/activate-codex.js --dry-run
node plugins/remem/scripts/activate-codex.js
```

## Distribution Channels

Currently published:

- Homebrew: `brew install majiayu000/tap/remem`
- GitHub Releases: prebuilt binaries for macOS and Linux on x64/arm64
- crates.io: `cargo install remem-ai --bin remem`
- npm: `npm install -g @remem-ai/remem`
- Source build: `cargo build --release`

Good next channels:

- apt/yum packages: useful later, after the binary install path and service
  story are stable across Linux distributions

## How It Works

remem uses host-specific hook strategies:

```
Claude Code workflow
        |
        |- SessionStart      -> Inject memories + preferences
        |- UserPromptSubmit  -> Register session, capture prompt + inject context
        |- PreToolUse(Bash)  -> Evaluate compiled preference rules
        |- PostToolUse       -> Capture tool operations (queued, <1ms)
        '- Stop              -> Summarize in background (~6ms return)

Codex workflow
        |
        |- SessionStart      -> Inject memories + preferences
        '- Stop              -> Summarize in background with Codex CLI
```

Codex does not install a high-frequency `PostToolUse(Bash)` observe hook by
default. Shell-heavy sessions must use the coalesced capture pipeline before
per-command capture is enabled again; otherwise Bash output can create an
unbounded backlog. Existing legacy hooks are also ignored unless
`REMEM_ENABLE_CODEX_BASH_OBSERVE=1` is set explicitly.

The capture pipeline starts with an append-only ledger:
`captured_events` stores raw hook/session evidence, `event_blobs` keeps large
payloads out of prompt-sized rows, and `extraction_tasks` coalesces work by
host/project/session instead of creating one LLM job per tool call. Curated
memory remains the promoted output of this pipeline, not the raw event itself.
Compatibility `events` projections share the canonical captured-event identity:
a hook retry reuses the same projection, while a projection failure rolls back
the capture and extraction task together so replay cannot create duplicates.

### Compiled preference rules

Compiled preference rules are disabled by default. Enable the worker/compiler
explicitly with `remem config set rule_compilation.enabled true`; SQLite
remains canonical, and the background worker rebuilds the derived
`<data-dir>/compiled_rules/<project-hash>.json` artifact. CLI overrides enqueue
that rebuild and take effect after it completes, without restarting the host:

```bash
remem rules list [--project <path>]
remem rules disable <rule-id>
remem rules enable <rule-id>
remem rules set-action <rule-id> warn
remem rules set-action <rule-id> block --host claude-code
```

On Claude Code, the installed `PreToolUse(Bash)` hook evaluates the artifact
before execution: `warn` is the default visible action, while `block` requires
explicit per-rule opt-in. `PostToolUse` remains capture-only and is not an
enforcement path. Codex has no supported pre-execution command hook, so remem
reports command enforcement as unsupported there and rejects Codex block-mode
claims. Missing, corrupt, or unsupported artifacts fail open and record an
error-level diagnostic instead of blocking the agent.

`remem doctor` reports whether compilation is enabled, artifact
presence/validity and rule count, compile time/error, the latest evaluation
error, and Claude/Codex enforcement capability without printing rule payloads.
GH-671 remains open because #813 still owns the exact global-owner filter and
exhaustive eligibility matrix.

## Remem vs Built-in `MEMORY.md`

Built-in memory files are enough when the context is small, stable, and worth
editing by hand: project rules, setup notes, and a short list of durable
preferences. Keep using them for facts that should be obvious at first glance.

Remem is meant for the parts that should not depend on manual upkeep:

- **Automatic capture and recall**: hooks summarize sessions into a SQLite
  memory store, while `remem search`, `remem show`, `timeline`, and MCP
  `get_observations` retrieve details on demand.
- **A bridge to native memory**: `remem sync-memory --cwd .` writes a compact
  `remem_sessions.md` entry for Claude Code native memory when that directory
  exists, with a `MEMORY.md` pointer and a size guard. Full detail stays in the
  database and is fetched with `remem search`.
- **A human-editable mirror**: `remem export --markdown --output
  ./remem-memory --project "$PWD"` writes one `.md` file per curated memory to
  an empty directory. After editing those files, `remem import markdown --source
  ./remem-memory` updates existing rows and rebuilds search, entity, embedding,
  and current-state indexes. Export refuses non-empty directories to avoid
  overwriting manual edits.
- **A git-diffable project memory pack**: maintainers can run
  `remem export --project "$PWD" --pack .remem-pack` and commit the generated
  `pack.json`, `memories.jsonl`, and `INDEX.md` files for active repo-owned
  startup memories. New contributors run `remem import --pack .remem-pack`
  from the same checkout to merge safe rows into their local store. Export
  re-runs the redaction scan and fails loudly on secret-like content; import
  dedups local rows, skips suppressed or invalidated local decisions, routes
  conflicts/quarantines to review, and marks imported memories with pack
  provenance visible in `remem why` and `remem doctor`.
- **Codex native memories import**: `remem import codex-memories --dry-run`
  plans a one-way, read-only import of Codex CLI rollout-summary memories
  (`~/.codex/memories/rollout_summaries`) and prints a plan digest; apply with
  `remem import codex-memories --expect-plan-digest <sha256>`. Records are
  treated as untrusted external content: they only land in the candidate
  review queue (`pending_review`/`quarantined`, secret-containing batches are
  blocked entirely), never directly in active memories, and the Codex source
  tree is never modified. Re-running the import is idempotent even if Codex
  renames its generated files.
- **Failure-loop learning**: raw transcripts that contain both concrete
  build/test failure evidence and an explicit "stop and challenge the
  hypothesis" style lesson feed an idempotent `failure` lesson before summary
  cooldown, duplicate, or skip exits.
- **Guarded Dream consolidation**: every model-generated topic key, type,
  title, content, no-merge reason, and conflict reason is scanned before Dream
  can persist or replace anything. The final title/content render is scanned as
  one surface too, so split-field instructions cannot bypass the boundary. A
  match becomes a project-scoped quarantined review candidate with a
  cryptographically version-bound source-cluster artifact; the original
  memories remain active, and newer semantic output invalidates an older review
  instead of silently rebinding it. Every decision rechecks source payload,
  TTL, current-state pointer, and suppression under its write lock; clean model
  output stays external trust and cannot rewrite a cluster-external dedup target.
- **Auditable candidate promotion**: candidate risk uses the closed
  `low`/`medium`/`high` rubric. Observation-derived low-risk facts are checked
  claim by claim against eligible source observations; supported negative facts
  and ordinary engineering uses of “token” are not rejected by a bare-word
  blacklist. Credential token variants, auth/authz state, destructive actions,
  generic imperative controls, outer negation, and ambiguous modal claims are
  deterministically review-gated even if a model labels them low risk. Directly
  supported failure-and-recovery lessons may promote, while preferences,
  procedures, secrets, instruction patterns, unsupported claims, and
  prospective or conditional claims remain review-gated or quarantined with an
  explicit block reason.
- **Governance and auditability**: `remem why <id>`, `remem govern --action
  stale --dry-run --json <id>`, `remem status --json`, and `remem usage --days
  14 --weeks 8` show why a memory is visible, what would change, store health,
  and memory-AI token/cost accounting.
- **Commit/session traceability**: successful explicit `git commit` results
  prove SHAs; trusted hook capture resolves those exact commits and links them
  to the durable session identity. Delayed workers and spill replay reuse that
  typed evidence; ordinary Stop events and missing evidence never guess from a
  later `HEAD`.
- **Current-memory accountability**: staleness labels, temporal facts,
  source-anchor checks, injection item audit rows, and citation/usage events
  show why a memory is current, stale, dropped, abstained, cited, or ignored.
- **User-context governance**: `remem user ...`, `remem memory suppress ...`,
  profile export, and non-retention policy checks keep personalized recall
  explicit instead of silently blending every user fact into every project.
- **Deterministic checks before claims**: local gates include
  `cargo test -q context::claude_memory --lib`, `cargo test -q eval::golden
  --lib`, `cargo test -q eval::governance --lib`, and `remem eval-e2e --json`.

Do not read this as a published claim that remem beats a carefully maintained
`MEMORY.md` on coding tasks. The flagship no-memory / remem / curated-file A/B
is still a separate benchmark requirement; until it is published, the honest
claim is capability coverage and reproducible local checks.

## Embedding Provider Configuration

Vector retrieval is controlled by the `[embeddings]` section in
`~/.remem/config.toml`:

```toml
[embeddings]
provider = "auto"         # auto | api | local | feature-hash | off
fallback = "feature-hash" # optional; omit for fail-closed
model = "text-embedding-3-small"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
model_dir = ""            # optional; defaults to REMEM_DATA_DIR/models
```

`auto` resolves in this order: a remem-specific API key, the verified
`multilingual-e5-small` local model, then the labeled `feature-hash` fallback.
An ambient `OPENAI_API_KEY` alone does not opt `auto` into remote calls.
remem never downloads model weights from a hook or search request; install the
default local model explicitly:

```bash
remem embedding download --model multilingual-e5-small
remem embedding status
remem embedding backfill --limit 1000
```

Automatic download currently supports only `multilingual-e5-small`, whose
evaluated immutable revision is pinned by remem. `bge-m3` remains recognized so
an already installed verified cache can still be loaded, but
`remem embedding download --model bge-m3` fails closed until remem publishes an
approved immutable BGE revision.

After the verified download, an `auto` configuration with no remem-specific
API key activates the local model. Run the idempotent backfill after any
provider/model switch—or after the downloaded artifact changes—so existing
memories gain vectors in the new model space. The active local model id carries
the verified artifact SHA-256, so vectors from different weight revisions are
never mixed even when their preset name and dimensions match.

Local ONNX initialization is a cold-start cost for short-lived commands. The
checked-in release-mode reference run measured about 5.43 seconds for provider
verification plus the first profile probe and 12 ms p95 for subsequent query
embeddings. Long-lived MCP/API processes reuse one process-wide session; the
report therefore records cold and warm latency separately instead of treating
the warm number as end-to-end startup time.

On Intel macOS (`darwin-x64`) release binaries, the `local` ONNX provider is
not compiled in because ONNX Runtime ships no prebuilt library for that
platform; embedding falls back to `feature-hash` (or `api` if configured), and
`remem status` / `remem doctor` report the provider state explicitly.

On Windows, local ONNX model operations accept only remem's default per-user
model root. `embeddings.model_dir` and non-default `REMEM_DATA_DIR` roots fail
closed: a shared/custom ancestor chain cannot provide the same reparse-point
and file identity guarantees. A worker-exported `REMEM_DATA_DIR` that resolves
to the default per-user directory remains supported. `auto` reports any real
override as unavailable and uses the labeled feature-hash fallback; explicit
`local` returns an actionable error. An older default model cache with
inherited/wide ACLs is not silently trusted or chmod-like rewritten; remove or
move that cache, then run `remem embedding download` again in the default
location.

Environment overrides keep the existing `REMEM_EMBEDDINGS_*` names, including
`REMEM_EMBEDDINGS_PROVIDER`, `REMEM_EMBEDDINGS_FALLBACK`,
`REMEM_EMBEDDINGS_MODEL`, `REMEM_EMBEDDINGS_MODEL_DIR`,
`REMEM_EMBEDDINGS_BASE_URL`, `REMEM_EMBEDDINGS_API_KEY`,
`REMEM_EMBEDDINGS_API_KEY_ENV`, `REMEM_EMBEDDINGS_DIMENSIONS`, and
`REMEM_EMBEDDINGS_TIMEOUT_SECS`.

`local` and `feature-hash` are separate provider states. `local` runs the
verified fastembed/ONNX model; `feature-hash` explicitly selects
`remem-local-feature-hash-v1`, the deterministic non-semantic fallback.
`provider = "off"` disables query embeddings, vector fusion, vector writes,
and embedding backfill. Existing stored vectors remain in SQLite but are
ignored while the provider is off.

The confidence gate admits a vector-only semantic fallback when no
claim-supported grounded result survives. If the query names an explicit
entity already stored in memory, that fallback must be directly bound to a
matching visible memory or connected through a specific entity shared by
already-fused visible candidates; common tags such as `API`, `remem`, `Claude`,
and `Team` cannot form that bridge. The candidate must still pass the predicate
claim check. Vector distance alone never turns an unsupported statement about
a known entity into an answer, while the constrained bridge preserves valid
owner-to-pager multi-hop recall.

`remem status --json` exposes an `embedding` object with configured provider,
active provider, active model id, degraded/disabled flags, and active-model
coverage. `remem doctor` reports unavailable configured providers, visible
fallback activation, low active-model coverage, and mixed model/dimension
profiles that need backfill.

## Search Architecture

remem uses multi-channel Reciprocal Rank Fusion (RRF) inspired by [Hindsight](https://github.com/vectorize-io/hindsight):

```
Query: "database encryption"
        |
   +----+------------------------------------+
   |       parallel retrieval channels       |
   +-----------------------------------------+
   | 1. FTS5 (BM25)   trigram + OR           |
   | 2. Entity Index  1600+ entities         |
   | 3. Temporal      "yesterday"/"last week" |
   | 4. LIKE fallback short tokens           |
   | 5. Trusted graph bounded expansion      |
   +-------------+---------------------------+
                 |
        contribution_i = weight_i / (60 + rank_i)
                         * (1 + normalized_signal_i)
                 |
             Top-K merged results
```

Entity, temporal, fact, LIKE fallback, and graph channels are rank-only, so
their `normalized_signal` is absent and the formula reduces to pure weighted
RRF; `rank_i` is one-based (`1, 2, ...`). FTS, vector, and usage channels
may add a calibrated `[0, 1]`
signal; an equal-score FTS result set also falls back to pure RRF. The usage
channel reranks only candidates the other channels already surfaced, using
`ln1p(access_count)` with a 30-day recency half-life. It ships at the
calibrated default weight `0.25` (GH-947, evidence in
`eval/weight-grid/report.json`); set `REMEM_USAGE_WEIGHT=0` to restore the
pre-rollout ranking exactly. Search
`--explain` reports each contribution's channel rank and total score. It also
reports the computed score identity
`final_score = sum(contribution.score) * post_fusion_score_factor`; the factor
captures post-fusion policies such as source-anchor demotion without adding
required fields to the public Rust explain structs.

Before claim and confidence gating, remem removes only validated temporal spans
and a restricted query-opening command scaffold. Entity numbers remain part of
the query. Malformed or identifier-shaped temporal text fails closed and stays
semantic instead of silently becoming a temporal filter.

Enhancements:

- Entity-index and trusted typed-graph expansion (bounded 2-hop retrieval)
- Project-scoped entity search (no cross-project leakage)
- CJK segmentation support
- Chinese-English synonym expansion
- Title-weighted BM25 (`bm25(fts, 10.0, 1.0)`)
- Content-hash deduplication via `topic_key`
- Multi-step retrieval guidance in MCP tool descriptions

## Benchmark Snapshot

### Public Artifact Suite (Directional Only)

The checked-in `eval/public` artifacts now separate memory-system capability
evidence from coding-agent outcome evidence. Reproduce the public verifier with:

```bash
cargo run -- bench verify --root eval/public --json-out /tmp/remem-bench-verify.json
cargo run -- bench report --root eval/public --json-out eval/public/reports/baseline.json --markdown-out eval/public/reports/baseline.md
```

The current directional report verifies 5 manifests, 5 reports, 45 run
artifacts, and 225 artifact files. It includes:

- `remem-code-memory`: 8 memory QA runs covering temporal/as-of answers, stale
  decision avoidance, conflicts, workstream continuity, prior bug root cause,
  architecture constraints, file/source anchors, and user-context relevance.
- `adversarial-policy`: 20 policy cases covering secrets, credentials, payment
  data, unsupported assistant claims, unapproved external sources, roleplay,
  negation, same-name repos, branch divergence, stale file anchors, unresolved
  conflicts, instruction injection, authority claims, opaque payloads, and a
  benign quoted-instruction control. Its `remem_default` condition runs real
  capture -> observation extraction -> candidate governance -> promotion and
  reads policy counts from the resulting SQLite state. Each artifact records
  that verification path plus the exact source/generated scanner split;
  direct-memory fixture conditions remain labeled comparative baselines.
- `issue385-smoke`: one committed coding-agent smoke run artifact with
  memory-contract fields for `remem` runs. The full `issue385-v1` fixture pack
  is referenced for dry-run reproduction, but it is not yet part of the
  verified public outcome report.

The report is intentionally labeled `directional_only_no_public_claim`. README
and release wording must stay directional and avoid broad outcome or
coding-task outcome claims until the public claim gate in
[`docs/release-lifecycle.md`](docs/release-lifecycle.md) passes.

### Isolated Coding-Agent Baseline (Internal, Not A Public Claim)

`eval/coding-bench/reports/baseline.json` contains an isolated 5-task,
3-runs-per-condition baseline generated with `codex-cli 0.142.1` and
`gpt-5.5`: `no_memory` resolved 2/15, `remem` resolved 15/15, and
`curated_file` resolved 15/15. This is useful engineering evidence, but it
predates the public 16-task v1 fixture pack and must be regenerated before it
supports stronger product claims.

### LoCoMo (Informational Only)

Full [LoCoMo](https://github.com/snap-research/locomo) benchmark (10 conversations, 1540 QA pairs after adversarial skip):

This snapshot is a historical footnote and is not a CI or release gate. Use the
golden retrieval eval for deterministic gating; LoCoMo remains useful only for
manual, informational comparison because the methodology is disputed.

| Config | Overall | Single-hop | Multi-hop | Temporal | Open-domain | Ingest | Model |
|---|---:|---:|---:|---:|---:|---|---|
| **v1 (fair)** | **56.8%** | 67.1% | 39.0% | 53.9% | 28.1% | per-turn | gpt-5.4 |
| **v2 (optimized)** | **62.7%** | 72.3% | 61.3% | 40.5% | 56.2% | session_summary | gpt-5.4 |

### Internal Eval (1777 real memories)

| Metric | Value |
|---|---:|
| MRR | 0.858 |
| Hit Rate@5 | 1.000 |
| Dedup rate | 1.0% |
| Project leak | 0% |
| Self-retrieval | 100% |

### Local QA Eval

```bash
python3 eval/local/run_local_eval.py --db ~/.remem/remem.db --n 20
```

| Metric | Score |
|---|---:|
| Overall | **85.0%** |
| Decision | 77.8% |
| Discovery | 87.5% |
| Preference | 100% |
| Source in top-20 | 90.0% |

Requires explicit `--db` plus `.env` with `OPENAI_API_KEY` (optional `OPENAI_BASE_URL`, `OPENAI_MODEL`).

### Sandboxed E2E Eval

```bash
remem eval-e2e
remem eval-e2e --json
```

Runs a deterministic coding-agent memory corpus through the real local REST API
boundary (`POST /api/v1/memories`, then `GET /api/v1/search`) with a temporary
`REMEM_DATA_DIR`. The default run removes the sandbox directory afterward, so it
does not touch `~/.remem` or other real memory data. Use `--keep-data-dir` when
you need to inspect the generated database.

## Token Usage And Cost Reporting

remem records an AI usage ledger for its own background extraction, summary,
compression, and promotion calls. The CLI can report daily and weekly token
usage and estimated cost:

```bash
remem usage --days 14 --weeks 8
remem usage --project /path/to/project --days 30 --weeks 12
```

The report includes calls, input tokens, cache tokens, output tokens, reasoning
tokens, total tokens, estimated USD cost, and a precision note. Usage rows are
tagged by source:

- `anthropic_usage`: provider-reported usage from the Anthropic Messages API
- `codex_log`: exact token counts parsed from the current `codex exec --json`
  `turn.completed.usage` event
- `text_estimate`: fallback estimate from prompt/response text length

Cost is an estimate, not an invoice. Historical rows may be text estimates or
may have been repriced from older rows that did not store the exact model.

## Memory AI Configuration

Memory AI execution is configured in `~/.remem/config.toml` (override path with
`REMEM_CONFIG`). Hooks pass only `--host`; the config maps each host to one
profile used by summarize, flush/extract, compress, and dream.

```bash
remem config path
remem config show
remem config set memory_ai.profiles.codex.model gpt-5.2
```

For normal model switching, prefer the higher-level `remem model` commands:

```bash
remem model current
remem model list
remem model use cheap
remem model use balanced --dry-run
remem model use gpt-5.2 --reasoning medium
remem model use haiku --host claude-code
remem model test
remem model test --live
remem model rollback
```

`remem model test` only validates the selected config unless `--live` is set.
`remem model use` saves a rollback backup before writing the config. Built-in
presets are Codex-focused; use explicit model names for Claude Code profiles.

Default Codex profile:

```toml
[memory_ai.hosts."codex-cli"]
memory_profile = "codex"
context_gate = "strict"
context_color = true
capture_adapter = "codex-cli"

[memory_ai.profiles.codex]
executor = "codex-cli"
model = "gpt-5.2"
path = "codex"
```

## Commands

```bash
remem install
remem uninstall
remem doctor
remem search "query"
remem search "query" --branch main --type decision --multi-hop --offset 10
remem search "query" --include-suppressed
remem search "query" --json
remem show <id>
remem show <id> --json
remem eval
remem eval-e2e --json
remem eval-local
remem backfill-entities
remem encrypt
remem api --port 5567
remem status
remem status --json
remem config show
remem config set memory_ai.profiles.codex.model gpt-5.2
remem rules list [--project <path>]
remem rules disable <rule-id>
remem rules enable <rule-id>
remem rules set-action <rule-id> warn
remem rules set-action <rule-id> block --host claude-code
remem model current
remem model list
remem model use balanced --dry-run
remem model use gpt-5.2 --reasoning medium
remem model use haiku --host claude-code
remem model test [--live]
remem model rollback
remem usage --days 14 --weeks 8
remem pending list-failed
remem pending list-failed --json
remem pending retry-failed --dry-run
remem pending list-extraction-ranges --id 308 --json
remem pending retry-extraction-ranges --id 308 --dry-run
remem pending retry-extraction-ranges --id 308
remem pending retry-extraction-ranges --id 308 --acknowledge-quarantine --dry-run
remem pending retry-extraction-ranges --id 308 --acknowledge-quarantine
remem pending retry-extraction-ranges --id 308 --acknowledge-quarantine --include-archived --dry-run
remem worker --once --replay-range-id 308 --acknowledge-quarantine --include-archived --profile claude
remem pending quarantine-extraction-ranges --id 308 --dry-run
remem pending migrate-legacy --dry-run
remem pending migrate-legacy
remem pending recover-archived --id 42 --dry-run
remem pending recover-archived --id 42
remem pending recover-archived --id 42 --host claude-code --dry-run
remem pending recover-archived --id 42 --host claude-code
remem pending purge-failed --dry-run --older-than-days 7
remem govern --action stale --dry-run --json <id>
remem review list
remem review approve <id>
remem review approve <id> --acknowledge-pattern <pattern_id>
remem review discard <id>
remem review edit <id> --text "updated memory"
remem procedures list
remem procedures list --project /repo/path --json
remem procedures export <id> --format runbook-md
remem procedures export <id> --format claude-skill --out remem-drafts
remem procedures export <id> --format codex-prompt --out remem-drafts --overwrite-generated
remem preferences list
remem preferences add "text"
remem preferences remove 42
remem memory suppress memory:123 --reason "not relevant anymore"
remem memory unsuppress memory:123 --reason "needed again"
remem memory feedback memory:123 --value not-relevant
remem memory suppressions list
remem user remember "For this repo, review specs before code"
remem user remember --scope repo --owner-key /repo/path --type goal "Ship remem user context"
remem user claims list
remem user claims why <id>
remem user claims edit <id> --text "updated claim"
remem user claims suppress <id>
remem user claims unsuppress <id>
remem user claims delete <id>
remem user summary show
remem user summary refresh
remem user summary edit --text "updated profile summary"
remem user summary sources
remem user profile export --format markdown --output profile.md
remem user recall "review the remem user context design"
remem user backfill --json --limit 100
remem user backfill --apply --json --limit 100
remem user review inbox
remem user review approve <id>
remem user review edit <id> --text "updated candidate"
remem user review reject <id>
remem user review suppress <id>
remem context --cwd .
remem cleanup --dry-run --json
remem cleanup --dry-run --json --archived-failures
remem cleanup
remem workstreams merge --project <path> --into <canonical_id> <duplicate_id>... --confirm
remem workstreams merge --project <path> --into <canonical_id> <duplicate_id>... --confirm --json
remem dream [--project X] [--profile NAME] [--dry-run]
remem install --target codex
remem mcp
remem sync-memory --cwd .
```

The worker schedules one database-global lifecycle cleanup at most once per
24-hour completed-attempt window. It first converges elapsed-TTL memories to
`stale`, then applies the same atomic retention policy as `remem cleanup`.
Audit events and live provenance are preserved. Automatic cleanup can never
purge archived failures; that hard-delete boundary still requires an explicit
positive `--archived-failures[=DAYS]` operator flag. `remem doctor` reports the
latest automatic success and failure independently.

### Legacy pending recovery

Current capture no longer writes or claims the retired
`pending_observations` queue. When no current extraction task is ready, an
ordinary worker can drain residual rows into the current capture/extraction
pipeline. `remem worker --once` admits at most one batch per process; a daemon
admits at most one batch every 60 seconds; each batch contains at most 25
oldest eligible rows. If current extraction work appears during legacy
preflight, a zero-progress yield keeps that admission available after current
work drains; a partial-progress yield consumes it.

Automatic candidates must have a known Claude Code or Codex host and be
pending, expired-processing, due transient failures, or controlled historical
archived transient failures. A success atomically records the current captured
event and extraction task, marks the legacy row migrated, and clears its old
failure/archive state. Any replay error rolls back current-pipeline writes,
records exponential backoff capped at 900 seconds, and stops that batch. The
bridge does not guess that a shared replay failure is row-local permanent.
Doctor keeps archived known-host transient rows visible during that backoff,
shows the earliest `next_retry_epoch`, and omits immediate `worker --once`
guidance until a row is due.
Rows already classified permanent and unknown-host rows remain available
through `remem pending` for inspection and explicit admin recovery; an unknown
host must be repaired before replay. The bridge does not restore the legacy
enqueue/claim API and never automatically deletes rows.

Archived failed legacy rows that are not eligible for the automatic bridge are
reported by doctor as `admin-required`. Doctor lists a bounded, oldest-first
set of those candidates directly, including each real ID, stored host, failure
class, archive time, and concrete `remem pending recover-archived` preview and
apply commands. A row stored with `host = unknown` is shown with both explicit
`--host claude-code` and `--host codex-cli` variants so the operator can choose
the correct identity. `recover-archived` rejects non-failed or non-archived
rows, replays only the requested ID in one transaction, and clears
failure/archive state only after the current event and extraction task commit;
an error leaves the source row unchanged.

Use the exact-ID extraction-range commands when recovering one known failure:
preview the retry or quarantine first, apply it only after the preview succeeds,
then query the same ID with `list-extraction-ranges --id <id> --json`. Exact
listing includes terminal `replayed` ranges and their linked replay task, so the
final range/task status and bounded error evidence remain auditable. `--id`
cannot be combined with batch `--project` or `--limit` filters and never falls
back to a sibling range. A quarantined range remains excluded from ordinary
exact and batch retry; restoring one requires the exact positive `--id` plus
`--acknowledge-quarantine`, first with `--dry-run` and then without it.
If the quarantined range has also archived, the pending command accepts
`--include-archived` only for a dual-confirmation dry-run. The write must use
the exact worker command with the same ID, both acknowledgements, and an
explicit `--profile`. That worker refuses to write while another worker holds
the singleton, atomically requeues and claims only the target task, and returns
partial, failed, timed-out, or interrupted attempts to archived quarantine
instead of exposing them to the ordinary daemon queue.

`remem procedures export` writes reviewable drafts for promoted procedure
memories. The default output is `remem-drafts/`; export refuses high-context
agent instruction paths such as `.claude/`, `.codex/`, `AGENTS.md`,
`CLAUDE.md`, repo `skills/`, `.agents/skills/`, and plugin `skills/` roots.
`--overwrite-generated` only replaces an unchanged remem-generated draft with a
matching export registry row.

`remem user ...` stores explicit user-context claims separately from
repo-scoped coding memories. Manual claims default to `owner_scope=user`,
`owner_key=user:default`, `source_kind=manual`, and `status=active`. Suppress
and delete commands change status without hard-deleting the audit row; default
claim lists exclude suppressed, deleted, expired, not-yet-valid, and restricted
claims.

Automatic user-context extraction can auto-promote only normal, low-risk
preference or constraint claims with stable claim keys, explicit user statement
sources, user-authored source events, and conservative text support. The default
auto-promote policy lowers only the confidence threshold from `0.9` to `0.7`:

```toml
[user_context.auto_promote]
min_confidence = 0.7
allowed_source_kinds = ["explicit_user_statement"]
require_text_support = true
strict = false
```

Set `strict = true` to restore the old `0.9` threshold while keeping the same
source and text-support requirements. Sensitivity, high risk, third-party
framing, non-user-authored source refs, missing keys, claim-key conflicts, and
non-retention matches remain hard review/no-retention gates in every mode.
`require_text_support = false` currently fails closed until queue support and
full source non-retention scanning are policy-aware.

`remem user profile export --format markdown` writes a derived, read-only
snapshot of the user profile remem would use. Without `--output` it prints to
stdout; with `--output profile.md` it creates a new file and refuses to
overwrite existing content. The snapshot names the SQLite database as the
source of truth, includes owner/project metadata, active summary provenance,
source ids, and active default-eligible claims. Default output excludes
suppressed, deleted, expired, future, personal, sensitive, and restricted
claims. Use `--include-suppressed`, `--include-sensitive`, `--include-inactive`,
`--include-deleted`, and `--include-manual-summaries` only for explicit audit;
audit rows are labeled with exclusion reasons and text remains redacted unless
all applicable audit gates are enabled.

`remem memory suppress` applies a default-read policy without deleting the
source row. Targets can be `memory:<id>`, `claim:<id>`, `topic:<key>`,
`entity:<name>`, `pattern:<text>`, or a bare memory id/topic key. Default
search, SessionStart context, profile-summary sources, preferences, lessons,
current-state lookup, MCP search, and REST search exclude active suppressions.
Use `--include-suppressed` on search when an audit needs to inspect suppressed
evidence explicitly. `remem why <id>` reports whether the memory is currently
suppressed and which policy matched it. `remem memory feedback` records
`relevant`, `not-relevant`, `harmful`, `stale`, or `too-noisy` events without
changing ranking by default.

`remem user recall <query>` retrieves task-aware user context on demand without
expanding SessionStart. It combines safe profile summaries, active
non-sensitive claims, repo memory, explicitly requested current-state keys,
active workstreams, and recent sessions into compact source-attributed context.
Default recall excludes suppressed, rejected, deleted, expired, future,
personal, sensitive, and restricted claims. Use `--include-sensitive` and
`--include-suppressed` only for explicit audit. Non-empty recall output includes
a usage policy telling agents to apply user context only when it materially
improves the answer, prefer invisible adaptation over memory narration, avoid
uncited profile inferences, and avoid inventing a profile when no context
applies.

`remem user backfill` migrates legacy user-scope preference memories into
governed user-context claims. Without `--apply`, it opens an existing database
read-only, reports candidate and skipped memory ids, and never creates,
migrates, or writes the store. With `--apply`, it inserts active preference
claims with `source_kind=preference_backfill` and JSON memory source refs while
leaving the source memory rows unchanged. `--limit <n>` bounds the source rows
processed, and `--json` emits the stable scriptable shape with
`converted[{memory_id, claim_id}]` and explicit skip reasons. The candidate set
matches visible legacy preference rows for `owner_scope=user`,
`owner_key=user:default`, `memory_type=preference`, and `status=active`; expired
or policy-suppressed rows are outside that visible set and do not become
candidates. Visible rows that fail guards or duplicate checks are reported in
`skipped[]` with row-level reasons such as `secret_like_content`,
`sensitivity_uncertain`, `instruction_pattern_unacknowledged:*`,
`text_too_long`, `duplicate`, or `governed_duplicate`. After apply, summary,
profile snapshot, and recall readers avoid showing the same preference as both a
legacy memory and a claim. Use `remem user claims why <claim_id>` to audit the
source `memory:<id>`, and use `remem user claims suppress <claim_id>` or
`remem user claims delete <claim_id>` to govern or roll back inserted claims;
the JSON report's `converted[].claim_id` gives the exact ids. Because the source
memory row is intentionally left unchanged, use `remem memory suppress` on
`memory:<id>` when the original legacy preference should also be hidden from
legacy memory readers.

`remem user review ...` governs review-gated user-context candidates before
they become active claims. `inbox` shows pending candidates with risk,
sensitivity, confidence, source preview, and block reason. `approve` applies a
candidate to active claims only when it has a stable claim key and non-empty
source refs; if an active claim with the same owner/type/key already exists,
remem either noops on an exact match or supersedes the old row instead of
appending a contradictory active claim. `edit` applies corrected text, key, or
metadata, while `reject` and `suppress` close candidates without activating
them.

`remem status` and `remem status --json` include a `user_context` block with
claim totals, active/suppressed/deleted claim counts, candidate totals,
pending-review and auto-promoted candidate counts, and pending block reasons.
Use `remem user claims why <id>`, `remem user claims suppress <id>`, and
`remem user claims delete <id>` to audit or roll back active claims created by
manual save, preference backfill, or auto-promotion.

### Dream Active-Memory Backfill (GH-990)

The v077 migration only adds the binding column and invariants; it never scans
or rewrites existing memories. To rehearse the pre-v076 Dream stock, run the
explicit command (dry-run is the default):

```bash
remem dream-backfill --dry-run --json
```

The report gives before-write stock, hit, no-hit, skipped, per-project, and
`plan_digest` counts. A hit is archived and placed in the existing quarantined
review queue; a no-hit only changes `source_trust_class` to `external_content`
and leaves recency timestamps unchanged. Nothing is written during rehearsal.

After reviewing the report, apply the exact rehearsal plan with:

```bash
remem dream-backfill --apply --expect-plan-digest <sha256>
```

`--apply` is the only mode that writes. The apply rechecks the complete stock
set and every row snapshot inside one immediate transaction; any plan drift
aborts all writes. Quarantine artifacts are immutable and bound to the retired
memory. Approving the resulting Dream candidate with its normal pattern and
provenance acknowledgements restores that same memory id in place; a changed
or missing target fails closed. Re-running after a successful apply produces an
empty plan.

### Raw Session Backfill

`remem ingest-sessions` batch-ingests Claude Code and Codex JSONL transcripts
into the raw archive without promoting them to curated memories:

```bash
remem ingest-sessions --json
remem ingest-sessions --since 2026-06-01 --root starlight=~/remote-sessions/starlight --json
```

Default scan roots are `~/.claude/projects` and `~/.codex/sessions`.
Additional `--root label=path` entries are required roots: a missing explicit
root is reported as a failed file so backfills do not silently do nothing. Each
transcript has a path-stable local identity ledger. Metadata IDs take
precedence over filename fallbacks, Stop and batch ingest use the same
identity, repeated identical turns retain separate occurrence ordinals, and
event-time provenance distinguishes transcript timestamps from ingest
fallbacks and legacy unknowns. Re-running the command is incremental and
idempotent; `--since`-skipped files receive an explicitly marked event-range
index for later bounded reconciliation, failed unindexed files remain stale,
and ambiguous identity claims fail visibly without rewriting raw rows.

Use raw time-window queries for recap or audit workflows that need original
chat turns rather than curated memories:

```bash
remem raw search "deployment decision" --since 2026-06-01 --until 2026-06-30 --json
remem raw sessions --since 2026-06-01 --until 2026-06-30 --sample 3 --json
remem raw messages --source-root local --project "/path/to/project" --session-id "<session-id>" --limit 500 --json
remem raw reconcile --since 2026-06-01 --until 2026-06-30 --json
```

`remem raw sessions` groups rows by source root, project, and session ID, and
reports total, user, and assistant message counts; it can include the first N
user-message samples per session. `remem raw messages` reads one exact
`(source_root, project, session_id)` tuple without truncating stored content.
It orders rows by `(created_at_epoch ASC, id ASC)`, defaults to 500 rows per
page, and returns an opaque `next_cursor` when `has_more` is true. The first
page freezes a maximum row ID; subsequent pages bind the cursor to the same
selectors and snapshot, so concurrent appends do not create duplicates,
omissions, or cross-session mixing. Invalid, stale, or selector-mismatched
cursors fail explicitly; a missing tuple returns a successful empty envelope.
`raw search`, `raw sessions`, `raw messages`, and `raw reconcile` open the
current schema read-only, so a writer lock does not trigger migration
contention and stale schemas fail with a migration diagnostic.

`raw reconcile` requires both bounds and compares stable per-occurrence
identities, not only aggregate counts. It validates the captured file
mtime/size tuple against the current identity ledger before reading, scans only
event-range candidates plus files with missing event time, scopes archive rows
to the requested source-root labels, and emits aggregate counts only—never
paths, projects, session IDs, hashes, or message text.
Timestamped records outside the inclusive UTC window are discarded before
classification. Meta/XML user rows remain in archive parity but are reported
as conversational exclusions; missing/fallback/legacy event time and malformed
records make `parity` false. Window-relevant identity conflicts also return a
non-zero status after the aggregate report is emitted.

A date-only `since` starts at `00:00:00` UTC, while a date-only `until`
includes that entire UTC day through `23:59:59`. MCP `search_raw` returns the
same JSON envelope and pagination fields as `remem raw search ... --json`.

### Scriptable JSON output

These commands emit one JSON object and no human text on stdout when `--json`
is set:

| Command | Stable top-level fields |
|---|---|
| `remem status --json` | `version`, `database`, `totals`, `embedding`, `raw_archive`, `capture_pipeline`, `promotion_funnel`, `legacy_surfaces`, `usage_feedback`, `pending_observations`, `review_queue`, `candidate_promotion`, `user_context`, `jobs`, `failure_lifecycle`, `worker_daemon`, `latest_session_memory_spend`, `today`, `top_projects` |
| `remem cleanup --dry-run --json` | `dry_run`, `retention_days`, `plan`, `applied`; archived failure purge counts stay zero unless `--archived-failures[=DAYS]` is supplied |
| `remem search ... --json` | `query`, `project`, `memory_type`, `limit`, `offset`, `branch`, `include_stale`, `include_suppressed`, `multi_hop_requested`, `explain_requested`, `count`, `has_more`, `next_offset`, `results`, `raw_hits`, `multi_hop`, `explain_details` |
| `remem ingest-sessions --json` | `scanned`, `skipped`, `ingested_messages`, `failed_files`, `partial_files` |
| `remem raw search ... --json` | `query`, `project`, `branch`, `role`, `limit`, `offset`, `since_epoch`, `until_epoch`, `count`, `has_more`, `next_offset`, `source_type`, `note`, `results` |
| `remem raw sessions ... --json` | `since_epoch`, `until_epoch`, `project`, `sample`, `count`, `sessions`; each session includes `message_count`, `user_message_count`, and `assistant_message_count` |
| `remem raw messages ... --json` | `source_type`, `source_root`, `project`, `session_id`, `order`, `limit`, `count`, `has_more`, `next_cursor`, `messages`; each message includes full `content` plus `id`, `role`, `source`, `branch`, `cwd`, and `created_at_epoch` |
| `remem raw reconcile ... --json` | `policy_version`, `since_epoch`, `until_epoch`, `transcript`, `archive`, `comparison`, `intentional_exclusions`, `parity` |
| `remem show <id> --json` | `found`, `id`, `memory` |
| `remem procedures list --json` | `project`, `limit`, `offset`, `count`, `procedures` |
| `remem memory suppress <target> --json` | `status`, `suppression` |
| `remem memory unsuppress <id-or-target> --json` | `status`, `count`, `suppressions` |
| `remem memory feedback <target> --json` | `status`, `feedback` |
| `remem memory suppressions list --json` | `count`, `suppressions` |
| `remem user remember --json` | `status`, `claim` |
| `remem user claims list --json` | `count`, `claims` |
| `remem user claims show <id> --json` / `remem user claims why <id> --json` | `found`, `claim` |
| `remem user claims edit <id> --json` | `status`, `previous_id`, `claim` |
| `remem user claims suppress <id> --json` / `unsuppress <id> --json` / `delete <id> --json` | `status`, `claim` |
| `remem user summary show --json` | `found`, `summary` |
| `remem user summary refresh --json` / `edit --json` | `status`, `summary` |
| `remem user summary sources --json` | `summary`, `included_claims`, `included_memories`, `included_activity_refs`, `dropped_claims` |
| `remem user recall <query> --json` | `query`, `project`, `task_intent`, `host`, `empty`, `context`, `usage_policy`, `included`, `dropped`, `diagnostics` |
| `remem user backfill --json` | `applied`, `limit`, `candidates`, `converted`, `skipped`, `message`; dry-run fills `candidates`, while `--apply` fills `converted[{memory_id, claim_id}]` for inserted claims |
| `remem user review inbox --json` | `count`, `candidates` |
| `remem user review approve <id> --json` / `edit <id> --json` | `status`, `action`, `candidate`, `claim` |
| `remem user review reject <id> --json` / `suppress <id> --json` | `status`, `candidate` |
| `remem workstreams merge --json` | `project`, `result` |
| `remem pending list-failed --json` | `project`, `limit`, `count`, `failed` |
| `remem pending list-extraction-ranges --id <id> --json` | `range` (including `id`, `status`, `attempts`, `last_error`, `replay_task_id`) and nullable `replay_task` (`id`, `status`, `attempts`, `last_error`); terminal `replayed` ranges remain queryable |
| `remem pending migrate-legacy --json` | `project`, `limit`, `count`, `migrated` |
| `remem govern ... --json` | `dry_run`, `action`, `reason`, `affected` |

## REST API

```bash
remem api --port 5567
TOKEN=$(cat ~/.remem/.api-token)
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:5567/api/v1/health
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:5567/api/v1/status
```

Library users who build the router directly should call
`remem::api::ensure_api_token()` before `remem::api::build_router(...)`.

The complete native web API surface is implemented in source version
`0.5.109`. remem-web should require a published `remem >= 0.5.109` release
before pointing installed-binary users at the full graph, candidate, or
rich-detail experience. Fast `/api/v1/health` and cached `/api/v1/status`
metadata are implemented in source version `0.5.112`. Clients should call
`/api/v1/capabilities` before enabling optional views. Suppression audit
opt-in with `include_suppressed=true` is implemented in source version
`0.5.113`; default search, browse, graph, and detail reads omit
policy-suppressed memories. On-demand user recall is implemented in source
version `0.5.114` through CLI, MCP, and `POST /api/v1/user/recall`.
User-context candidate review inbox and apply lifecycle commands are
implemented in source version `0.5.115`. Guarded automatic user-context
candidate extraction from session rollups is implemented in source version
`0.5.116`; it creates review candidates from captured user conversations and
session summaries, and auto-promotes only normal, low-risk explicit user
preference or constraint statements cited to and supported by user-authored
source events. Source capture, bounded rollup follow-up ranges, stale review
guards, edited candidate audit persistence, and claim-key conflict review gates
are tightened in source version `0.5.117`. Failed bounded follow-up retries and
transactional auto-promotion conflict rechecks are tightened in source version
`0.5.118`. User-context candidate extraction non-retention rules are tightened
in source version `0.5.122`; transient, speculative, unsafe,
assistant-authored, or unapproved external-source content does not enter the
candidate queue. Source version `0.5.125` tightens post-review external-source
attribution and third-party subject edge cases while preserving valid
user-stated workflow preferences. Source versions `0.5.183` through `0.5.186`
make user-context auto-promote configuration-driven: the default lowers only the
confidence threshold to `0.7`, `strict = true` restores the old `0.9` threshold,
existing hard gates remain review/no-retention gates, and `remem status` reports
user-context claim/candidate counts and pending block reasons.
Source version `0.6.6` implements the GH-880 safe console API: candidate
detail/evidence and idempotent safe review, five independently gated safe read
resources, and recoverable memory archive/restore. Installed clients must wait
for a published `v0.6.6` release and require the exact capability/endpoint-map
bundle; the staged `unreleased` source manifest is not release evidence.

Use `/api/v1/health` as the cheap liveness probe and `/api/v1/capabilities` for
feature detection. Use `/api/v1/status` for dashboard counters no more
frequently than the returned `cache.ttl_secs`; use
`/api/v1/status?refresh=true` only for explicit refresh actions.

### Stable core endpoints

| Endpoint | Method | Description |
|---|---|---|
| `/api/v1/health` | GET | Cheap authenticated liveness and API readiness |
| `/api/v1/status` | GET | Cached queue state and counts with cache metadata |
| `/api/v1/capabilities` | GET | Feature and endpoint detection for native clients |
| `/api/v1/search?query=&project=&type=&limit=&offset=&branch=&multi_hop=&include_suppressed=` | GET | Search memories |
| `/api/v1/memory?id=&include_suppressed=` | GET | Get one memory |
| `/api/v1/memories?project=&type=&scope=&status=&branch=&q=&limit=&offset=&include_suppressed=` | GET | Canonical memory browse endpoint |
| `/api/v1/memories/{id}?include_suppressed=` | GET | Rich memory detail with entities and edges |
| `/api/v1/memories` | POST | Save memory |
| `/api/v1/user/recall` | POST | Task-aware user-context recall with source and drop reasons |

### Web read-model endpoints

| Endpoint | Method | Description |
|---|---|---|
| `/api/v1/stats` | GET | Product stats for local dashboards |
| `/api/v1/candidates?project=&status=&limit=&offset=` | GET | List compact memory candidates |
| `/api/v1/candidates/{id}` | GET | Safe candidate detail, evidence/provenance, action-specific review decision, and Dream review token when applicable |
| `/api/v1/candidates/{id}/review/approve` | POST | Versioned, audited, idempotent safe approval; Dream quarantine requires the current pattern and provenance-token acknowledgements |
| `/api/v1/candidates/{id}/review/reject` | POST | Versioned, audited, idempotent safe rejection |
| `/api/v1/candidates/{id}/review/edit` | POST | Versioned, audited, idempotent safe edit-and-approve |
| `/api/v1/candidates/{id}/approve` | POST | Approve a pending memory candidate; quarantined candidates require `acknowledge_pattern` |
| `/api/v1/candidates/{id}/reject` | POST | Reject a pending memory candidate |
| `/api/v1/candidates/{id}/edit` | POST | Edit and approve a pending memory candidate |
| `/api/v1/graph?project=&limit=&include_suppressed=` | GET | DB-backed entity graph read model |
| `/api/v1/observations[/{id}]` | GET | Safe observation list/detail with typed cursor |
| `/api/v1/sessions[/{id}]` | GET | Safe session list/detail with typed cursor |
| `/api/v1/workstreams[/{id}]` | GET | Safe workstream list/detail with typed cursor |
| `/api/v1/events[/{id}]` | GET | Safe event metadata list/detail without raw content |
| `/api/v1/tasks[/{id}]` | GET | Safe task list/detail without raw payload/error text |
| `/api/v1/memories/{id}/archive` | POST | Recoverably archive an active memory |
| `/api/v1/memories/{id}/restore` | POST | Restore only the current exact Web archive |

Permanent Web delete is intentionally unavailable. `memory_delete=false` and
the capability endpoint map contains no delete key.

### Compatibility aliases

| Endpoint | Method | Description |
|---|---|---|
| `/api/v1/memories/list` | GET | Compatibility alias for `/api/v1/memories` |
| `/api/v1/memory?id=&include_suppressed=` | GET | Legacy compact single-memory endpoint |

Run the local native API smoke test against a built binary with:

```bash
scripts/smoke_native_web_api.sh
```

## Security

- SQLCipher encryption at rest (`remem encrypt`)
- Data directory permissions (`0700`)
- Key file permissions (`0600`)
- REST API binds localhost only (`127.0.0.1`) and requires
  `Authorization: Bearer $(cat ~/.remem/.api-token)`
- API token file permissions (`0600`)

### SQLite runtime tuning

Runtime connections use WAL mode, foreign keys, a 5-second busy timeout,
in-memory temporary storage, and a 64 MiB SQLite page-cache target per
connection. The cache is demand-driven rather than preallocated, but concurrent
connections can multiply both memory use and the time decrypted pages remain in
RAM.

Two environment variables provide strict, fail-closed overrides:

- `REMEM_SQLITE_CACHE_KIB` accepts an integer from `1` through `1048576`.
  SQLite interprets it as a KiB cache target for each connection.
- `REMEM_SQLITE_SYNCHRONOUS` accepts only `full` or `normal`
  (case-insensitive). The default is `full`.

`normal` is an explicit latency/durability tradeoff. With WAL it preserves
database consistency and committed transactions across an application-process
crash, but a system crash or power loss can lose recent committed transactions.
Keep the default `full` when power-loss durability matters. Temporary storage
is always forced to memory because SQLCipher does not guarantee encryption for
file-backed SQLite temporary storage.

Reproduce the encrypted release-mode A/B latency harness with:

```bash
cargo test --release --test search_latency_benchmark sqlite_tuning_encrypted_release_ab_reports_latency -- --ignored --exact --nocapture --test-threads=1
```

### Plaintext residue diagnostics

The `Plaintext residue` check in `remem doctor` recursively inspects regular
files throughout the managed `REMEM_DATA_DIR` tree by content, including
custom backup outputs with arbitrary names or no extension. It never follows
symlinks and, on Windows, rejects every filesystem reparse point before
recursive descent. A file shorter than the SQLite header inside the managed
`backups/` subtree is reported as an incomplete inspection; unrelated short
operational files elsewhere are ignored unless their names identify them as
database artifacts. Configured key and live-database sidecar paths retain their
existing exclusions. A strictly validated internal Hugging Face cache snapshot pointer
is omitted because its same-repository blob is a regular file that the tree
scan inspects independently; any other artifact symlink makes the inspection
incomplete. The check is read-only and never deletes data. When it finds a
plaintext copy while the live database is confirmed encrypted, it reports
`Fail` and `remem doctor` exits with code 2. If the live database is plaintext
or its encryption state cannot be confirmed, the finding is `Warn`. An entry
or candidate file that cannot be inspected because of an I/O error is reported
and prevents the check from reporting `Ok`.

Handle a finding according to its status:

- `Fail` means doctor has confirmed that the live database is encrypted and a
  plaintext copy exists. Run `remem status` to verify that the live database
  opens, run `remem admin backup` to create a new encrypted backup, and only
  then manually delete the listed plaintext copies or retain them solely in
  encrypted storage.
- For `Warn` with a plaintext live database, first check whether
  `REMEM_DATA_DIR/remem.db.bak` or `REMEM_DATA_DIR/remem.db.enc` is blocking
  encryption. If either exists, verify that `REMEM_DATA_DIR/backups/` is an
  actual directory and not a symlink, choose unused destination names there,
  and move each blocker without overwriting another file. Then run
  `remem encrypt` and use `remem status` to confirm that the live database
  opens. Run `remem doctor` and verify that the Plaintext residue detail says
  the live database is encrypted and readable; this check is expected to remain
  `Fail` while the preserved copies are still present. Then run
  `remem admin backup` to create a new encrypted backup, manually delete the
  older plaintext copies or retain them solely in encrypted storage, and rerun
  `remem doctor` until Plaintext residue passes.
- For `Warn` caused by a missing or unverified live database, key problems, or
  I/O errors, do not back up or delete anything. First repair the live database,
  key, or readability problem. Continue with backup and disposal only after
  `remem status` succeeds and `remem doctor` confirms encryption.

Moving a copy outside `REMEM_DATA_DIR` only removes it from this scan; it does
not protect the data. Remem does not promise secure erasure of manually deleted
files.

## Architecture Docs

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for full internals and data flow.

## Uninstall

```bash
remem uninstall
rm -rf ~/.remem
```

## The Agent Infra Stack

This project is one layer of an open-source stack for running coding agents (Claude Code, Codex) as serious infrastructure. Every piece works standalone; together they close the loop:

`remem` is the **Remember** layer — memory that survives the session. It pairs naturally with `keepline`, which keeps the sessions themselves from getting lost.

| Layer | Project | What it does |
|---|---|---|
| Extend | [claude-skill-registry](https://github.com/majiayu000/claude-skill-registry) | Discover and search community Claude Code skills |
| Extend | [spellbook](https://github.com/majiayu000/spellbook) | Cross-runtime skills for Claude Code, Codex, and multi-agent workflows |
| Trust | [argus](https://github.com/majiayu000/argus) | Static install-time scanner for supply-chain attacks (npm / PyPI / crates.io) |
| Trust | [vibeguard](https://github.com/majiayu000/vibeguard) | Rules, hooks, and guards against hallucinated or unverified agent changes |
| Remember | [remem](https://github.com/majiayu000/remem) **◀ you are here** | Local-first persistent memory for Claude Code and Codex sessions |
| Orchestrate | [harness](https://github.com/majiayu000/harness) | Rust agent orchestration platform — rules, skills, GC, observability |
| Route | [litellm-rs](https://github.com/majiayu000/litellm-rs) | High-performance Rust AI gateway — 100+ LLM APIs via OpenAI format |
| Keep | [keepline](https://github.com/majiayu000/keepline) | Session command center — monitor, recover, never lose agent work |

---

## License

MIT
