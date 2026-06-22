# Current Memory Contracts Technical Spec

Status: Current contract
Issues: Refs #381, #383, #384, #385, #390

## Current Implementation Truth

This spec is anchored on the runtime state of `origin/main` when the contract
was written. The repository already has the following production paths:

- migration registry and convergence tests under `src/migrate/`;
- durable memory rows in `memories`;
- temporal facts in `memory_facts`;
- transaction-time invalidation via `invalidated_at_epoch`;
- source episode time via `reference_time_epoch`;
- state-key current resolution and operation logs;
- memory lifecycle and graph/conflict edges;
- context injection gate rows and per-item audit rows;
- citation and usage feedback rows;
- source-anchor staleness labels;
- doctor, status, REST, MCP, CLI, plugin, and local app surfaces.

Implementation work for this contract must extend those paths. It must not
introduce a replacement storage model or a second retrieval stack.

## Contract Inventory

| Contract | Existing owner | Notes |
|---|---|---|
| Durable curated memory | `memories` plus `src/memory/` | `content` remains canonical body; `search_context` is rebuildable metadata. |
| Current slot | `memory_state_keys`, `current_state` | Returns active/unexpired rows and conflict states. |
| Operation audit | `memory_operation_log` | Records add/update/noop/defer/conflict decisions. |
| Memory lifecycle relations | `memory_edges` | Handles supersedes, duplicate, merge, split, conflict links. |
| Temporal facts | `memory_facts` | Stores event-valid fact windows, learned time, invalidation, evidence, confidence. |
| Cross-node graph | `graph_edges` | Typed trusted/diagnostic graph relation, not a replacement memory store. |
| Output injection gate | `context_injections` | De-dup and output-mode state keyed by host and injection key. |
| Per-item injection audit | `context_injection_items` | Append-only injected/dropped/abstained item decisions. |
| Citation feedback | `memory_citation_events` | One event per assistant message hash/source. |
| Usage feedback | `memory_usage_events` | Links cited memory IDs to injected item rows. |
| Source-anchor staleness | `MemoryStalenessLabel` | Exposed through API/search/context metadata. |
| A/B outcome evidence | `issue385-coding-agent-ab` | Separate end-to-end benchmark contract. |

## Storage Contracts

### `memories`

`memories` is the durable curated memory table. New columns may be added through
forward migrations when they extend this model, but no replacement table may
become the canonical memory store.

Required invariants:

- Active/current retrieval must honor status, expiry, scope, owner, branch, and
  state-key filters.
- Direct save and candidate promotion must write through migration-managed
  schemas and audited operation paths.
- `reference_time_epoch` must be populated on insert/update. Backfills may use
  best available source time, but code must not silently default historical
  imports to `now()` when source time is available.
- `last_accessed_epoch` and `access_count` are usage feedback counters, not a
  replacement ranking model.

### `memory_facts`

`memory_facts` is the temporal fact contract.

Required invariants:

- `valid_from_epoch` / `valid_to_epoch` are event-validity bounds.
- `learned_at_epoch` is transaction-time learning.
- `invalidated_at_epoch` is transaction-time invalidation.
- `source_memory_id`, `source_observation_id`, and `source_event_ids` preserve
  provenance.
- Current fact filters must exclude stale or invalidated facts.
- Fact retrieval must not require an external graph database.

Implementation work should add writer coverage, retrieval coverage, and doctor
metrics before adding new fact-like tables.

### State, Operation, And Edge Tables

`memory_state_keys` owns mutable current slots. `memory_operation_log` owns the
audit trail. `memory_edges` owns memory lifecycle relations. `graph_edges` owns
typed cross-node relations.

Required invariants:

- A conflict must be auditable as an operation or edge, not hidden as a normal
  update.
- Trusted graph edges must include source event ids, candidate or operation
  provenance, confidence, and reason.
- Graph candidates remain reviewable until promoted into trusted graph edges.
- Conflict writers must fail closed when they cannot prove endpoint identity.

### Injection And Usage Tables

`context_injections` and `context_injection_items` must remain separate:

- `context_injections` records output-level gate state.
- `context_injection_items` records item-level accountability.

`memory_citation_events` and `memory_usage_events` must remain linked to
injected items:

- `memory_citation_events` records whether the assistant provided the citation
  line and whether parsed ids matched injected memories.
- `memory_usage_events` records matched cited memories and links them back to
  `context_injection_items`.
- Missing or unmatched citations are data and must be recorded.

## Retrieval And Ranking Contract

Search is a fused channel plan. This spec does not replace it.

Required behavior:

- Default search remains current-only and suppressed-excluding unless the caller
  opts in to stale or suppressed rows.
- Temporal and fact channels may improve recall only through existing search
  plan channels or narrowly scoped extensions.
- Source-anchor demotion must apply before final pagination or explain output.
- Staleness labels must be visible in search/detail/list API items and explain
  records.
- Usage ranking stays shadow/off by default. The current default usage weight
  is `0.0`; any default change requires:
  - a committed weight-grid report;
  - eval-gates baseline and threshold updates;
  - no regression in abstention or scored query count;
  - a handoff to the coding-agent A/B benchmark.

## Context Injection Contract

Every SessionStart/UserPromptSubmit context render must produce enough audit
evidence to reconstruct the decision.

Required behavior:

- Rendered memory items include provenance and staleness metadata.
- Dropped memory items include a drop reason when the system considered but did
  not render them.
- Abstention creates an audit row when no relevant memory should be rendered.
- Suppressed duplicate output is visible through `context_injections`.
- Source-anchor errors are logged and rendered as error labels or surfaced as
  load errors; they are not silently treated as `untracked`.

## Observability Contract

The current observability data exists across doctor, CLI status, REST status,
context gate commands, API search responses, and plugin/app wrappers. The next
implementation slice should provide one structured contract for app and API
consumers.

The structured observability payload should include:

- schema version and generated timestamp;
- runtime version and migration status;
- capture liveness and capture-drop summary;
- promotion funnel summary;
- context injection summary:
  - latest runs by host/project/session;
  - output modes;
  - emit/suppress counts;
  - item counts by status, channel, and drop reason;
  - source-anchor distribution;
- usage feedback summary:
  - citation events;
  - citation line present count;
  - parsed, matched, inserted, unmatched, and no-citation counts;
  - usage events;
  - injected-but-unused memory/session counts;
- temporal fact summary:
  - table exists;
  - total rows;
  - retrieval-eligible rows;
  - invalidated, expired, orphan, and unlinked counts;
- staleness summary:
  - tracked;
  - untracked;
  - verify-before-trust;
  - error;
  - fresh/aging/old age buckets;
- queues and worker state;
- warnings with stable codes and suggested actions.

`doctor --json` should expose structured fields beyond string details:

```json
{
  "code": "memory_usage_feedback_low_match_rate",
  "severity": "warn",
  "metrics": {
    "citation_events": 10,
    "parsed_events": 8,
    "matched_events": 4,
    "usage_events": 4
  },
  "actions": ["verify injected citation contract", "run eval-gates"],
  "scope": "project"
}
```

The local app should consume this structured contract and must not scrape human
CLI output or read SQLite directly.

## API Contract

Existing search/list/detail/show API responses expose `staleness` on
`MemoryItem`. That shape is part of the public contract:

- `status`;
- `age`;
- `source_anchor`;
- `label`;
- optional `error`.

Source-anchor failures must return a structured failure such as
`staleness_source_anchor_failed` or an explicit `source_anchor=error` label,
depending on the route. Silent fallback to `untracked` is not acceptable.

Current-state responses must expose the same staleness/source-anchor contract
for all user-facing memory refs:

- `current` answer;
- conflict memory refs;
- chronological history refs.

If a current-state route cannot compute source-anchor staleness for one of
those refs, it must return a structured error field on that ref rather than
omitting staleness or downgrading to `untracked`.

If a new `/api/v1/observability` endpoint is added, it should be read-only and
loopback/local-app oriented. It must not expose raw archive content unless the
caller explicitly opts in through a separate raw-preview contract.

## Host, Plugin, And App Boundaries

Host adapters may own host-specific activation and rendering. They may not own
memory truth.

Required boundaries:

- Claude Code adapter owns Claude-specific hooks and prompt surfaces.
- Codex adapter/plugin owns Codex plugin packaging, version-matched runtime
  resolution, and explicit activation helpers.
- The plugin manifest is not a silent hook installer.
- The local app is a loopback UI over public APIs.
- Apps SDK connector ids and `.app.json` entries may be added only after a real
  app/connector id exists.
- JavaScript app code must not reimplement staleness, temporal, retrieval,
  ranking, promotion, or conflict semantics.

## Evaluation Contract

This spec uses deterministic eval gates. It depends on
`issue385-coding-agent-ab` for end-to-end agent outcome proof.

Required gates for implementation slices:

### Always

```bash
cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json
```

### Search, temporal, current-state, or staleness changes

```bash
cargo test -q current_state --lib
cargo test -q temporal --lib
cargo test -q staleness --lib
cargo test -q retrieval::search --lib
```

### API label or public error changes

```bash
cargo test -q api --lib
cargo test --test api_public
```

### Usage or ranking changes

```bash
cargo run -- eval-weight-grid --json-out /tmp/remem-weight-grid.json
cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json
```

Usage ranking default changes also require an explicit handoff to the
coding-agent A/B benchmark report. A deterministic retrieval win alone is not
enough to claim coding-agent outcome improvement.

### Plugin or local app boundary changes

```bash
node --test plugins/remem/scripts/remem-runtime.test.js \
  plugins/remem/apps/remem/server.test.js \
  npm/remem/scripts/install.test.js
```

## Implementation Plan

### Slice 1: Contract Documentation

- Add this spec and index it as a current contract.
- Link it from implementation issues that touch temporal facts, source-anchor
  staleness, usage ranking, injection audit, or local app observability.

### Slice 2: Structured Observability

- Add a structured doctor/check payload or `/api/v1/observability`.
- Reuse existing query paths before adding new tables.
- Include injection, usage, fact, staleness, promotion, capture, and worker
  metrics.
- Add API and doctor tests for stable warning codes.

### Slice 3: Contract Gates

- Extend deterministic evals for:
  - current-state statuses;
  - current-state staleness/source-anchor labels on current, conflict, and
    history refs;
  - invalidated and expired fact exclusion;
  - as-of fact retrieval;
  - tracked/untracked/verify-before-trust/error staleness labels;
  - injection item audit rows;
  - citation/usage event linking.
- Add or update eval-gates thresholds only with committed baseline evidence.

### Slice 4: Usage Feedback Shadow Ranking

- Report usage channel scores with `usage > 0` in shadow/eval mode.
- Keep default usage weight at `0.0` until eval reports justify changing it.
- If the default changes, update benchmark and README claim boundaries.

### Slice 5: Local App Consumption

- Update the local app to consume structured observability data.
- Keep write actions explicit and confirmation-gated.
- Do not add Apps SDK manifest `apps` entries until connector id ownership is
  resolved.

## Forbidden Changes Without A Follow-Up Spec

- Creating `memory_versions` as a replacement source of truth.
- Creating a second migration lineage or side database for runtime memory truth.
- Splitting crates for architecture appearance before typed contracts stabilize.
- Adding JSON-RPC/UDS as a new main IPC channel.
- Enabling usage ranking by default without eval and benchmark evidence.
- Letting the local app or plugin bypass Rust runtime contracts.
- Treating source-anchor errors as `untracked`.

## Open Technical Decisions

- Whether structured observability should be implemented as REST-only,
  doctor-only, or both.
- Whether staleness `error` labels should be injected into context by default or
  block injection for some hosts.
- Which exact eval-gates slices should own usage-feedback thresholds.
- Whether current-state conflict metrics belong in the same observability
  payload or in a separate governance endpoint.
