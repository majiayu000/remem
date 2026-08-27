# remem documentation

This page routes readers to the current source of truth for each part of
remem. Start with the root [README](../README.md) for installation and daily
use. Use this page when you need configuration, operations, architecture, API,
or evidence details.

Each route below identifies its authority. **Current executable** means the
installed CLI or runtime reports the effective behavior. **Current
architecture** is the module and data-flow source in `docs/ARCHITECTURE.md`.
**Current contract** means an entry marked current in the
[spec index](specs/README.md). **Current guide** explains shipped behavior but
does not override executable help, current architecture, or a current
contract. Dated research and historical design references are labeled
explicitly and are not runtime truth.

## Start here

- [English README](../README.md)
- [简体中文 README](../README.zh-CN.md)
- [Changelog](../CHANGELOG.md)
- [Contributing](../CONTRIBUTING.md)
- [Security policy](../SECURITY.md)

## Installation and host integration

| Topic | Primary route | Authority |
|---|---|---|
| Quick installation and verification | [Root README](../README.md#install-in-five-minutes) | Current guide; verify with `remem doctor` |
| Install channels, upgrades, platforms, and PATH drift | [Installation and upgrade guide](installation.md) | Current guide |
| Codex plugin runtime and explicit hook activation | [Plugin README](../plugins/remem/README.md) | Current runtime guide |
| SessionStart smoke test | [SessionStart context smoke](sessionstart-context-smoke.md) | Current verification guide |
| SessionStart injection and cache stability | [Architecture](ARCHITECTURE.md) and [cache-stability contract](specs/cache-stable-injection/PRODUCT.md) | Current architecture and contract |
| Cursor host evidence and limitations | [Cursor contract research](research/cursor-hooks-contract-2026-07-23.md) | Dated research snapshot; verify with `remem doctor` |
| Plugin target design | [Codex plugin complete design](spec-codex-plugin-complete-design.md) | Historical target design; use the Plugin README for shipped behavior |
| Release channels and artifact policy | [Release lifecycle](release-lifecycle.md) | Current guide |

Host behavior changes quickly. `remem doctor` and `remem install --dry-run`
are the current executable checks for a machine; design documents explain why
the integration behaves that way.

## Configuration

| Topic | Primary route | Authority |
|---|---|---|
| Memory AI hosts, profiles, models, and executors | [CLI workflow](../README.md#configure-memory-ai-and-retrieval), `remem config show`, and `remem config --help` | Current executable |
| SessionStart budgets and selection | [Context-budget contract](specs/context-budget-config/PRODUCT.md) | Current contract |
| Local semantic embeddings | [Local embedding product contract](specs/local-semantic-embedding/PRODUCT.md) | Current contract |
| SQLite cache and durability policy | [SQLite tuning contract](specs/GH949/PRODUCT.md) | Current contract |
| Usage and cost reporting | [Memory usage guide](memory-usage-guide.md) and `remem usage --help` | Current guide and executable |

Use `remem config show` to inspect the effective runtime configuration. Fresh
Claude host configuration defaults `context_gate` to `auto`; explicit stored
values remain visible in that output. Use command-specific `--help` output for
the exact current option set.

## Using and governing memory

| Topic | Primary route | Authority |
|---|---|---|
| MCP search, detail retrieval, saving, and workstreams | [Memory usage guide](memory-usage-guide.md) | Current guide |
| Curated-memory lifecycle and stored status | [Memory lifecycle](memory-lifecycle.md) | Current implementation guide; visibility authority is the [legacy-unverified contract](specs/legacy-unverified-context/PRODUCT.md) |
| Temporal and as-of facts | [Temporal facts](temporal-facts.md) | Current implementation guide |
| Procedure export | [Procedural memory](procedural-memory.md) | Current implementation guide |
| User claims, profiles, recall, and review | [User-context contract](specs/user-context-layer/PRODUCT.md) | Current contract |
| Raw transcript ingestion and queries | [Raw-session contract](specs/raw-session-ingestion/PRODUCT.md) | Current contract |
| Review queue throughput and filters | [Review-queue contract](specs/review-queue-throughput/PRODUCT.md) | Current contract |
| Failed work, replay, and archived recovery | [Failure-lifecycle contract](specs/failure-lifecycle/PRODUCT.md) | Current contract |
| Project memory packs | [Project memory pack contract](specs/project-memory-pack/PRODUCT.md) | Current contract |
| Compiled preference rules | [Preference-rule contract](specs/preference-rule-compilation/PRODUCT.md) | Current contract |

The CLI is the canonical command reference:

```bash
remem --help
remem <command> --help
```

This repository does not duplicate every CLI flag in the landing page because
hand-maintained command inventories drift as the product evolves.

## Interfaces

| Interface | Primary route | Authority |
|---|---|---|
| MCP tools and structured output | [MCP metadata contract](specs/GH981/PRODUCT.md) | Current contract |
| Experimental Context Bundle compiler | [Context Bundle contract](specs/GH932/PRODUCT.md) | Current contract; interface remains experimental |
| Experimental retrieval-plan compiler | [Retrieval-router contract](specs/GH934/PRODUCT.md) | Current contract; interface remains experimental |
| Local authenticated REST API | [Web API contract](specs/SPEC-web-api.md) | Current contract |
| Codex plugin MCP wrapper | [Plugin README](../plugins/remem/README.md) | Current runtime guide |
| Native/local app prototype | [Plugin app section](../plugins/remem/README.md#local-app-surface) | Current runtime guide; surface remains a prototype |

REST clients should call `/api/v1/capabilities` before enabling optional
views. MCP clients should inspect the served tool descriptors instead of
assuming an older tool count or schema.

## Architecture and data flow

- [Current architecture overview and module map](ARCHITECTURE.md)
- [Current graph implementation guide](graph-contract.md)
- [Current memory ownership and visibility](ARCHITECTURE.md#memory-scope-project-vs-global) and [legacy-unverified quarantine contract](specs/legacy-unverified-context/PRODUCT.md)
- [Current SessionStart compiler architecture](ARCHITECTURE.md#4-context-injection-sessionstart--context) and [Context Bundle contract](specs/GH932/PRODUCT.md)
- [Current workstream operations](memory-usage-guide.md#workstreams), [architecture](ARCHITECTURE.md), and [identity-continuity contract](specs/workstream-identity-continuity/PRODUCT.md)
- [Current spec index and lifecycle status](specs/README.md)

The spec index must be read before treating an older spec as pending work.
Many historical specifications document completed or superseded behavior.

## Security and operations

| Topic | Primary route | Authority |
|---|---|---|
| Vulnerability reporting and security policy | [SECURITY.md](../SECURITY.md) | Current policy |
| Memory poisoning and quarantine | [Poisoning-defense contract](specs/memory-poisoning-defense/PRODUCT.md) | Current contract |
| Log rotation and health diagnostics | [Log hardening contract](specs/log-rotation-hardening/PRODUCT.md) | Current contract |
| Failure recovery and retention | [Failure-lifecycle contract](specs/failure-lifecycle/PRODUCT.md) | Current contract |
| Database tuning and durability | [SQLite tuning contract](specs/GH949/PRODUCT.md) | Current contract |

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
