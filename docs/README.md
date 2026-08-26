# remem documentation

This page routes readers to the current source of truth for each part of
remem. Start with the root [README](../README.md) for installation and daily
use. Use this page when you need configuration, operations, architecture, API,
or evidence details.

## Start here

- [English README](../README.md)
- [简体中文 README](../README.zh-CN.md)
- [Changelog](../CHANGELOG.md)
- [Contributing](../CONTRIBUTING.md)
- [Security policy](../SECURITY.md)

## Installation and host integration

| Topic | Current document |
|---|---|
| Quick installation and verification | [Root README](../README.md#install-in-five-minutes) |
| Install channels, upgrades, platforms, and PATH drift | [Installation and upgrade guide](installation.md) |
| Codex plugin runtime and explicit hook activation | [Plugin README](../plugins/remem/README.md) |
| SessionStart smoke test | [SessionStart context smoke](sessionstart-context-smoke.md) |
| SessionStart injection and cache stability | [Architecture](ARCHITECTURE.md) and [current cache-stability contract](specs/cache-stable-injection/PRODUCT.md) |
| Cursor host evidence and limitations | [Cursor contract research](research/cursor-hooks-contract-2026-07-23.md) |
| Plugin target design | [Codex plugin complete design](spec-codex-plugin-complete-design.md) |
| Release channels and artifact policy | [Release lifecycle](release-lifecycle.md) |

Host behavior changes quickly. `remem doctor` and `remem install --dry-run`
are the current executable checks for a machine; design documents explain why
the integration behaves that way.

## Configuration

| Topic | Current document |
|---|---|
| Memory AI hosts, profiles, models, and executors | [Memory AI config](spec-memory-ai-config.md) |
| SessionStart budgets and selection | [Current context-budget contract](specs/context-budget-config/PRODUCT.md) |
| Local semantic embeddings | [Local embedding product contract](specs/local-semantic-embedding/PRODUCT.md) |
| SQLite cache and durability policy | [SQLite tuning contract](specs/GH949/PRODUCT.md) |
| Usage and cost reporting | [Memory usage guide](memory-usage-guide.md) and `remem usage --help` |

Use `remem config show` to inspect the effective runtime configuration. Use
the command-specific `--help` output for the exact current option set.

## Using and governing memory

| Topic | Current document |
|---|---|
| MCP search, detail retrieval, saving, and workstreams | [Memory usage guide](memory-usage-guide.md) |
| Curated-memory lifecycle | [Memory lifecycle](memory-lifecycle.md) |
| Temporal and as-of facts | [Temporal facts](temporal-facts.md) |
| Procedure export | [Procedural memory](procedural-memory.md) |
| User claims, profiles, recall, and review | [User-context contract](specs/user-context-layer/PRODUCT.md) |
| Raw transcript ingestion and queries | [Raw-session contract](specs/raw-session-ingestion/PRODUCT.md) |
| Review queue throughput and filters | [Review-queue contract](specs/review-queue-throughput/PRODUCT.md) |
| Failed work, replay, and archived recovery | [Failure-lifecycle contract](specs/failure-lifecycle/PRODUCT.md) |
| Project memory packs | [Project memory pack contract](specs/project-memory-pack/PRODUCT.md) |
| Compiled preference rules | [Preference-rule contract](specs/preference-rule-compilation/PRODUCT.md) |

The CLI is the canonical command reference:

```bash
remem --help
remem <command> --help
```

This repository does not duplicate every CLI flag in the landing page because
hand-maintained command inventories drift as the product evolves.

## Interfaces

| Interface | Current document |
|---|---|
| MCP tools and structured output | [MCP metadata contract](specs/GH981/PRODUCT.md) |
| Experimental Context Bundle compiler | [Context Bundle contract](specs/GH932/PRODUCT.md) |
| Experimental retrieval-plan compiler | [Retrieval-router contract](specs/GH934/PRODUCT.md) |
| Local authenticated REST API | [Web API contract](specs/SPEC-web-api.md) |
| Codex plugin MCP wrapper | [Plugin README](../plugins/remem/README.md) |
| Native/local app prototype | [Plugin app section](../plugins/remem/README.md#local-app-surface) |

REST clients should call `/api/v1/capabilities` before enabling optional
views. MCP clients should inspect the served tool descriptors instead of
assuming an older tool count or schema.

## Architecture and data flow

- [Architecture overview and module map](ARCHITECTURE.md)
- [Graph contract](graph-contract.md)
- [Memory ownership and routing](spec-memory-ownership-routing.md)
- [Context compiler design](spec-context-compiler.md)
- [Workstream design](workstream-design.md)
- [Current spec index and lifecycle status](specs/README.md)

The spec index must be read before treating an older spec as pending work.
Many historical specifications document completed or superseded behavior.

## Security and operations

| Topic | Current document |
|---|---|
| Vulnerability reporting and security policy | [SECURITY.md](../SECURITY.md) |
| Memory poisoning and quarantine | [Poisoning-defense contract](specs/memory-poisoning-defense/PRODUCT.md) |
| Log rotation and health diagnostics | [Log hardening contract](specs/log-rotation-hardening/PRODUCT.md) |
| Failure recovery and retention | [Failure-lifecycle contract](specs/failure-lifecycle/PRODUCT.md) |
| Database tuning and durability | [SQLite tuning contract](specs/GH949/PRODUCT.md) |

`remem doctor` is the primary operator entrypoint. It reports install drift,
schema and key failures, plaintext residue, queue health, compiler state, and
other actionable diagnostics without printing memory payloads.

## Evaluation and public evidence

- [Evaluation overview](../eval/README.md)
- [Public artifact suite](../eval/public/README.md)
- [Coding-agent benchmark](../eval/coding-bench/README.md)
- [LoCoMo informational snapshot](../eval/locomo/README.md)
- [Cross-host benchmark](../eval/cross-host/README.md)
- [Public benchmark contract](specs/public-memory-benchmark/PRODUCT.md)

Only checked-in, verifier-valid artifacts support public benchmark wording.
Historical or local-only numbers must remain labeled and must not be promoted
to product outcome claims.

## Research and comparisons

- [Memory-tool ecosystem survey](research/claude-memory-mcp-ecosystem-2026-03.md)
- [Memory systems comparison](memory-systems-comparison.md)
- [Competitive analysis](competitive-analysis.md)
- [Retrieval research](ref/memory-retrieval-research-2026-05-24.md)

Research documents are dated snapshots. Check upstream projects before using
them as current feature comparisons.
