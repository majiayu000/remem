# remem: Local-first Coding Agent Memory for Claude Code and OpenAI Codex

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

![Remem Memory terminal demo](assets/remem-demo.gif)

## What You Get

- Claude Code, OpenAI Codex, and Codex CLI remember project decisions across sessions.
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

`remem install` can auto-detect existing Claude Code and Codex CLI config
directories. On first-time setups, use `--target codex`, `--target claude`, or
`--target all` so remem can create the selected config files.

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
tools exposed by `remem mcp`, including `search`, `get_observations`,
`save_memory`, `workstreams`, and `timeline`.

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
        |- UserPromptSubmit  -> Register session, flush stale queues
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
- **Failure-loop learning**: raw transcripts that contain both concrete
  build/test failure evidence and an explicit "stop and challenge the
  hypothesis" style lesson feed an idempotent `failure` lesson before summary
  cooldown, duplicate, or skip exits.
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
provider = "local"        # api | local | feature-hash | off
fallback = "feature-hash" # optional; omit for fail-closed
model = "text-embedding-3-small"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
model_dir = ""            # future local model cache; defaults under REMEM_DATA_DIR
```

Environment overrides keep the existing `REMEM_EMBEDDINGS_*` names, including
`REMEM_EMBEDDINGS_PROVIDER`, `REMEM_EMBEDDINGS_FALLBACK`,
`REMEM_EMBEDDINGS_MODEL`, `REMEM_EMBEDDINGS_MODEL_DIR`,
`REMEM_EMBEDDINGS_BASE_URL`, `REMEM_EMBEDDINGS_API_KEY`,
`REMEM_EMBEDDINGS_API_KEY_ENV`, `REMEM_EMBEDDINGS_DIMENSIONS`, and
`REMEM_EMBEDDINGS_TIMEOUT_SECS`.

`local` and `feature-hash` are separate provider states. Until the local
semantic model runtime lands, both use `remem-local-feature-hash-v1`; selecting
`feature-hash` explicitly labels the non-semantic fallback. `provider = "off"`
disables query embeddings, vector fusion, vector writes, and embedding
backfill. Existing stored vectors remain in SQLite but are ignored while the
provider is off.

`remem status --json` exposes an `embedding` object with configured provider,
active provider, active model id, degraded/disabled flags, and active-model
coverage. `remem doctor` reports unavailable configured providers, visible
fallback activation, low active-model coverage, and mixed model/dimension
profiles that need backfill.

## Search Architecture

remem uses 4-channel Reciprocal Rank Fusion (RRF) inspired by [Hindsight](https://github.com/vectorize-io/hindsight):

```
Query: "database encryption"
        |
   +----+------------------------------------+
   |          4 parallel channels            |
   +-----------------------------------------+
   | 1. FTS5 (BM25)   trigram + OR           |
   | 2. Entity Index  1600+ entities         |
   | 3. Temporal      "yesterday"/"last week" |
   | 4. LIKE fallback short tokens           |
   +-------------+---------------------------+
                 |
        RRF score = sum(1 / (60 + rank_i))
                 |
             Top-K merged results
```

Enhancements:

- Entity graph expansion (2-hop multi-hop retrieval)
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

The current directional report verifies 4 manifests, 4 reports, 25 run
artifacts, and 125 artifact files. It includes:

- `remem-code-memory`: 8 memory QA runs covering temporal/as-of answers, stale
  decision avoidance, conflicts, workstream continuity, prior bug root cause,
  architecture constraints, file/source anchors, and user-context relevance.
- `adversarial-policy`: 15 non-retention cases covering secrets, credentials,
  payment data, unsupported assistant claims, unapproved external sources,
  roleplay, negation, same-name repos, branch divergence, stale file anchors,
  and unresolved conflicts.
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
remem pending migrate-legacy --dry-run
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
raw row keeps the source-root label and the transcript event timestamp, and
re-running the command is incremental and idempotent.

Use raw time-window queries for recap or audit workflows that need original
chat turns rather than curated memories:

```bash
remem raw search "deployment decision" --since 2026-06-01 --until 2026-06-30 --json
remem raw sessions --since 2026-06-01 --until 2026-06-30 --sample 3 --json
```

`remem raw sessions` groups rows by source root, project, and session ID, and
can include the first N user-message samples per session.

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
| `remem raw sessions ... --json` | `since_epoch`, `until_epoch`, `project`, `sample`, `count`, `sessions` |
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
| `/api/v1/candidates/{id}/approve` | POST | Approve a pending memory candidate; quarantined candidates require `acknowledge_pattern` |
| `/api/v1/candidates/{id}/reject` | POST | Reject a pending memory candidate |
| `/api/v1/candidates/{id}/edit` | POST | Edit and approve a pending memory candidate |
| `/api/v1/graph?project=&limit=&include_suppressed=` | GET | DB-backed entity graph read model |

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
