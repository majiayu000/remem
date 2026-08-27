# remem: Local-first memory for Claude Code and OpenAI Codex

[![MCP Toplist](https://mcptoplist.com/badge/io.github.majiayu000%2Fremem.svg)](https://mcptoplist.com/server/io.github.majiayu000%2Fremem)

> Stop re-explaining your project every new coding-agent session.

Language: **English** | [简体中文](README.zh-CN.md)

`remem` automatically captures, distills, searches, and injects engineering
memory across Claude Code and OpenAI Codex CLI sessions. Decisions,
bug-fix rationale, project patterns, and preferences stay available through
hooks, MCP, CLI, and a localhost REST API.

[![CI](https://github.com/majiayu000/remem/actions/workflows/ci.yml/badge.svg)](https://github.com/majiayu000/remem/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/majiayu000/remem?sort=semver)](https://github.com/majiayu000/remem/releases/latest)
[![crates.io](https://img.shields.io/crates/v/remem-ai)](https://crates.io/crates/remem-ai)
[![npm](https://img.shields.io/npm/v/%40remem-ai%2Fremem)](https://www.npmjs.com/package/@remem-ai/remem)
[![License MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

![Remem recall demo showing a new session picking up an earlier bug fix](assets/remem-recall-demo.gif)

*A new Claude Code session recalls the earlier root cause, commit, and open
TODO with memory citations and no re-explaining.*

## What remem gives you

- Automatic session capture and background LLM distillation.
- Project-scoped recall across Claude Code and Codex using one local store.
- Searchable decisions, bug fixes, architecture notes, preferences, and raw
  session evidence.
- Source attribution, staleness labels, suppression, review queues, and
  injection audits.
- SQLite with SQLCipher encryption by default for fresh installs.
- MCP, CLI, and authenticated localhost REST access from one Rust runtime.

remem prioritizes memory quality. Automatic capture is the primary path;
manual `save_memory` calls supplement it when a decision needs to be recorded
immediately.

## Install in five minutes

### Homebrew

```bash
brew install majiayu000/tap/remem
REMEM_INSTALL_BINARY="$(brew --prefix remem)/bin/remem" remem install --target codex
```

Use `--target claude` for Claude Code. `--target all` configures every known
host, including Cursor where its v1 renderer is supported.

### Standalone installer

```bash
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | env REMEM_NO_CONFIG=1 sh
~/.local/bin/remem install --target codex
```

### npm or Cargo

```bash
npm install -g @remem-ai/remem
# or
cargo install remem-ai --bin remem

remem install --target codex
```

GitHub Releases: prebuilt binaries for macOS and Linux on x64/arm64, with
published checksums. Use one canonical `remem` executable on `PATH`;
`remem doctor` warns when hooks and terminals resolve different copies.

For channel-specific upgrades, platform boundaries, PATH drift, and manual
install notes, read the [installation and upgrade guide](docs/installation.md).
The broader [documentation guide](docs/README.md) links plugin and operational
material.

## Verify the installation

Restart the selected coding agent, then run:

```bash
remem doctor
remem status
remem search "last decision"
```

A healthy Claude Code or Codex installation injects relevant project memory at
SessionStart and queues durable session distillation at Stop. `remem doctor`
checks the schema, encryption key, database, hooks, MCP registration, worker,
and common install-path drift.

For a focused, read-only view of current-memory truth:

```bash
remem doctor truth --cwd .
```

## Host support

| Capability | Claude Code | Codex CLI | Cursor v1 |
|---|---|---|---|
| MCP memory tools | Yes | Yes | Yes on macOS/Linux |
| SessionStart injection | Yes | Yes | Not supported |
| Automatic session memory | Yes | Yes, Stop-based and low-noise | Not enabled by the v1 installer |
| Tool-event capture | Installed hooks | No high-frequency Bash hook by default | Runtime command exists; no installed hook |
| Compiled command-rule enforcement | Optional warn/block on Bash | Not supported | Not supported |
| Windows | Supported | Supported | Not supported |

Cursor's v1 installer registers MCP only. The verified `observe` and
`summarize` runtime commands exist, but `remem install --target cursor` does
not install automatic capture hooks or SessionStart injection.

The repository also includes a Codex plugin wrapper. See
[plugins/remem/README.md](plugins/remem/README.md) for local plugin runtime and
explicit hook activation instructions.

## Why use remem alongside built-in memory

Built-in `MEMORY.md`, `CLAUDE.md`, and agent instruction files are ideal for a
small set of stable facts that should always be visible. remem covers the
engineering history that is too large, dynamic, or evidence-heavy to maintain
by hand.

| Need | Built-in files | remem |
|---|---|---|
| Stable project rules | Excellent | Supported |
| Automatic session capture | Manual upkeep | Hook-driven |
| Search older rationale | Limited by loaded text | Curated and raw search |
| Branch, time, and staleness handling | Manual | Built in |
| Provenance and injection audit | Git history | Database-backed audit |
| Review, suppression, and lifecycle governance | Manual edits | First-class commands |

Use both. Keep concise rules in native files and let remem retain the long tail
of decisions, failures, evidence, and changing project state.

The broader ecosystem comparison lives in the dated
[memory-tool survey](docs/research/claude-memory-mcp-ecosystem-2026-03.md).

## How it works

```text
Claude Code / Codex hooks
          |
          v
append-only captured_events ledger
          |
          v
coalesced background extraction and session rollup
          |
          v
governed candidates -> curated memories + workstreams + raw archive
          |
          v
FTS, entity, temporal, vector, graph, and optional local rerank retrieval
          |
          v
budgeted, source-attributed SessionStart context
```

Hooks return quickly after durable capture or queueing. Background workers
perform extraction, candidate governance, compression, retrieval enrichment,
and lifecycle cleanup. MCP, CLI, REST, and SessionStart share the same local
store and governance model, but apply surface-specific eligibility policies.
Explicit search is an inspection and recovery surface, so it may return
labeled `legacy-unverified` memories; default SessionStart and CurrentTruth
exclude those rows and record the reason.

Generated memory is treated as untrusted until it passes source-support,
secret, instruction-pattern, scope, and lifecycle checks. Unsafe content is
dropped or routed to review with a diagnosable reason.

For module ownership and current data flow, read
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

The experimental MCP `context_bundle` tool exposes the versioned, budgeted
compiler to explicit callers. The experimental `remem context-plan` command
prints a request-specific retrieval plan. These opt-in interfaces are tracked
by the [Context Bundle](docs/specs/GH932/PRODUCT.md) and
[retrieval-router](docs/specs/GH934/PRODUCT.md) contracts.

## Everyday workflows

### Recall and inspect

```bash
remem search "database encryption"
remem search "deployment decision" --branch main --explain
remem show <memory-id>
remem why <memory-id>
remem current <state-key>
```

Agents can use MCP `search` for compact results, then `get_observations` for
selected details. Use raw recall only when curated memory misses exact
transcript evidence:

```bash
remem raw search "exact phrase" --since 2026-06-01 --json
```

### Review and govern

```bash
remem review list
remem review approve <candidate-id>
remem memory suppress memory:<id> --reason "no longer relevant"
remem govern --action stale --dry-run --json <id>
```

Mutating governance commands expose previews, explicit confirmations, or
review boundaries according to their risk. Run `remem <command> --help` for
the current contract instead of relying on a copied command inventory.

### Configure memory AI and retrieval

```bash
remem config show
remem model current
remem model use balanced --dry-run
remem embedding status
remem embedding download --model multilingual-e5-small
remem embedding backfill --limit 1000
```

`auto` embedding mode stays local unless a remem-specific API key is selected.
The verified local model is optional; the labeled feature-hash fallback remains
available. The second-stage local reranker is also optional and disabled until
configured.

See the [memory-AI config contract](docs/spec-memory-ai-config.md),
[local embedding contract](docs/specs/local-semantic-embedding/PRODUCT.md), and
`remem config`, `remem embedding`, or `remem reranker` help for details.

### Share or edit memory outside the database

```bash
remem sync-memory --cwd .
remem export --markdown --output ./remem-memory --project "$PWD"
remem export --project "$PWD" --pack .remem-pack
```

Markdown mirrors are human-editable. Project memory packs are deterministic,
git-committable exports with provenance-aware import and quarantine behavior.
See the [memory usage guide](docs/memory-usage-guide.md) and
[project memory pack contract](docs/specs/project-memory-pack/PRODUCT.md).

## Evidence and benchmarks

The checked-in public suite separates memory-system capability evidence from
coding-agent outcome evidence. Verify it locally with:

```bash
cargo run -- bench verify --root eval/public --json-out /tmp/remem-bench-verify.json
```

The current public report is deliberately labeled
`directional_only_no_public_claim`. The historical isolated coding baseline is
useful engineering evidence, but its preloaded-memory condition is not
comparable with the current SessionStart retrieval path.

Reproduction commands, artifact schemas, claim boundaries, and current gates
live in:

- [eval/README.md](eval/README.md)
- [eval/public/README.md](eval/public/README.md)
- [eval/coding-bench/README.md](eval/coding-bench/README.md)

README claims intentionally exclude unsealed local metrics that have no
checked-in report.

## Security and privacy

- Fresh installs create a SQLCipher-encrypted database and private key file.
- The data directory and key use restrictive per-user permissions.
- The REST API binds to `127.0.0.1` and requires a bearer token.
- Hook-captured event previews are redacted before durable storage.
- Memory candidates and injected content pass secret and poisoning defenses.
- `remem doctor` reports encryption, plaintext residue, schema, and audit
  failures without printing memory payloads.

Read [SECURITY.md](SECURITY.md) for reporting and security policy. Operational
contracts for [SQLite tuning](docs/specs/GH949/PRODUCT.md) and
[memory-poisoning defense](docs/specs/memory-poisoning-defense/PRODUCT.md) are
kept outside the landing page.

## REST API

```bash
remem api --port 5567
TOKEN=$(cat ~/.remem/.api-token)
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:5567/api/v1/health
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:5567/api/v1/capabilities
```

Clients should feature-detect through `/api/v1/capabilities`. The current
endpoint and compatibility contract is maintained in
[docs/specs/SPEC-web-api.md](docs/specs/SPEC-web-api.md).

## Documentation

Use [docs/README.md](docs/README.md) as the jump page for installation,
configuration, memory lifecycle, retrieval, governance, API, plugin,
operations, architecture, and benchmark material.

The most common destinations are:

- [Architecture and data flow](docs/ARCHITECTURE.md)
- [Memory usage guide](docs/memory-usage-guide.md)
- [Memory lifecycle](docs/memory-lifecycle.md)
- [Codex plugin](plugins/remem/README.md)
- [REST API contract](docs/specs/SPEC-web-api.md)
- [Current spec index](docs/specs/README.md)
- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)

## Uninstall

Preview and remove host hooks and MCP registration without deleting memory:

```bash
remem uninstall --dry-run
remem uninstall
```

The encrypted database remains in the configured `REMEM_DATA_DIR`. Back it up
before manually deleting that directory if data removal is intended. Ordinary
file deletion removes remem's local data but does not guarantee secure erasure
from filesystem snapshots, backups, or the underlying storage media.

## License

MIT
