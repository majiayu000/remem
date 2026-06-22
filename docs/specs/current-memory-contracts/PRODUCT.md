# Current Memory Contracts Product Spec

Status: Current contract
Issues: Refs #381, #383, #384, #385, #390

## Problem

remem already has the runtime pieces that should define current memory truth:
curated memories, temporal facts, state keys, graph/conflict edges, context
injection audits, citation usage events, source-anchor staleness labels, doctor
checks, REST endpoints, MCP tools, and deterministic eval gates.

The remaining product risk is not the absence of a more ideal architecture. The
risk is that these existing pieces can drift into parallel meanings:

- a new schema beside `memories` / `memory_facts`;
- a new retrieval path beside the current search plan;
- a new App or plugin interpretation beside the Rust runtime;
- a usage feedback feature that records data but never affects evidence gates;
- temporal or staleness labels that are visible in one surface but silent in
  another.

This spec defines the product contract for closing those gaps by converging the
existing implementation paths. It explicitly avoids a second rewrite.

## Goal

Make remem's existing memory system auditable enough that a user can answer:

1. What memory is current for this project, branch, task, and reference time?
2. Why was this memory injected, dropped, suppressed, demoted, or abstained?
3. Was an injected memory later cited by the agent?
4. Did temporal facts and source-anchor staleness affect the result?
5. Which doctor, API, and eval signals prove the system is healthy?

The contract must be strong enough to support the coding-agent A/B benchmark in
`docs/specs/issue385-coding-agent-ab/`, while staying inside the current runtime
architecture.

## Non-Goals

- Do not split the repository into `remem-core`, `remem-storage`,
  `remem-retrieve`, or `remem-bench` crates as part of this contract.
- Do not introduce `memory_versions` or any other replacement source of truth
  for durable memories.
- Do not add JSON-RPC over Unix sockets, named pipes, or another IPC surface
  unless a later spec proves MCP/REST/CLI cannot satisfy a concrete requirement.
- Do not make the Apps SDK or local app read SQLite directly or recompute
  staleness, temporal truth, ranking, or promotion semantics in JavaScript.
- Do not change default ranking weights, especially usage feedback, without a
  deterministic eval report and a benchmark handoff.
- Do not use this spec to close the coding-agent A/B benchmark issue. This spec
  is a prerequisite quality contract; the A/B runner and report remain governed
  by `issue385-coding-agent-ab`.

## Product Contract

### Current Memory Truth

`memories` remains the durable curated memory table. Other tables may extend,
annotate, audit, or route memories, but they must not become a second durable
memory source.

The current-memory answer is the result of these existing contracts working
together:

- `memories`: durable curated memory content and lifecycle status.
- `memory_state_keys`: current slot resolution for stable topics.
- `memory_operation_log`: audited add/update/noop/defer/conflict decisions.
- `memory_edges`: memory-to-memory lifecycle links such as supersedes,
  duplicates, merge, split, and conflicts.
- `memory_facts`: temporal fact layer attached to source memories and evidence.
- `graph_edges`: typed cross-node graph evidence, including promoted conflict
  and file-touch relations.
- `context_injections`: output-level de-duplication and gate state.
- `context_injection_items`: append-only per-item injection decisions.
- `memory_citation_events` and `memory_usage_events`: Stop-time usage feedback
  linked back to injected memories.
- staleness labels: visible age and source-anchor trust status.

Any new table that touches current memory semantics must state which existing
contract it extends and why the existing table cannot hold the data.

### Temporal Truth

remem must keep event validity separate from transaction-time knowledge:

- `valid_from_epoch` / `valid_to_epoch` describe when a fact is true in the
  remembered world.
- `learned_at_epoch` describes when remem learned the fact.
- `invalidated_at_epoch` describes when remem learned that a fact stopped being
  current.
- `reference_time_epoch` describes the episode/source time used for capture,
  extraction, memories, and temporal provenance.

No product surface may collapse these into a single `created_at` or
`updated_at` interpretation.

### Staleness And Source Anchors

Every user-facing memory result must carry a staleness label with:

- memory lifecycle status;
- age label;
- source-anchor state;
- a human-readable label;
- an error field when staleness could not be computed safely.

The source-anchor states are:

- `tracked`: source code/file evidence is anchored and no later overlapping
  change was detected.
- `verify-before-trust`: later overlapping source evidence exists; the memory
  must be demoted and rendered with a visible warning.
- `untracked`: no source anchor is available.
- `error`: staleness could not be computed and the failure must be visible.

Search, detail, list, current-state, and context injection surfaces must not
silently turn `error` into `untracked`.

### Injection Accountability

Every context emission must be explainable at two levels:

- output-level: `context_injections` records the host, injection key, mode,
  output hash, emit count, suppress count, and duplicate-injection gate state.
- item-level: `context_injection_items` records each memory-like item as
  `injected`, `dropped`, or `abstained`, including channel, rank/order,
  provenance, staleness, and drop reason.

A context that emits no memory because the system abstained must still leave an
auditable reason. A suppressed duplicate emission must still be visible as a
gate decision.

### Usage Feedback

The usage feedback product contract is evidence-first:

- injected memories include a stable citation contract;
- Stop-time assistant output is parsed into `memory_citation_events`;
- matched citations produce `memory_usage_events` linked to
  `context_injection_items`;
- access counters may be updated only from those usage events or explicit
  detail reads;
- missing citations are recorded as `no_citation` rather than ignored.

Usage feedback must start as a reporting and shadow-ranking signal. It must not
change default ranking unless deterministic evals and the coding-agent A/B
benchmark show no regression.

### Host And App Boundaries

Claude Code, Codex, MCP, REST, CLI, and the local app may have different
activation and rendering details, but they must consume the same Rust runtime
contracts.

- Host adapters own host-specific hooks, activation, and prompt shape.
- The Codex plugin owns plugin packaging, explicit runtime resolution, and
  activation helpers.
- The local app owns UI and user interaction over public APIs.
- The Rust runtime owns memory truth, temporal facts, staleness, ranking,
  usage feedback, doctor checks, and migrations.

The local app must not silently install hooks, write high-context host config,
or implement a separate memory truth engine.

## User-Facing Outcomes

After this contract is implemented and verified:

- `remem doctor` can say whether automatic capture, temporal facts, promotion,
  injection, usage feedback, and source-anchor staleness are working.
- REST/API consumers can inspect the same health categories without scraping
  human CLI output.
- The local app can show why a memory is trustworthy, stale, unused, cited, or
  excluded.
- The A/B benchmark can run its `remem` condition against a deterministic,
  auditable memory contract instead of a best-effort black box.

## Dependencies

- `docs/specs/issue385-coding-agent-ab/` provides the end-to-end agent outcome
  benchmark. This spec is a prerequisite contract for that benchmark's `remem`
  condition, not a replacement.
- Existing golden, injection, extraction, eval-gates, and weight-grid evals
  remain deterministic quality gates.
- Spec lifecycle governance applies: spec-only work uses `Refs`, while
  implementation issues close only after code, tests, docs, and smoke checks
  land.

## Acceptance Criteria

- The specs index marks this directory as a current contract.
- Runtime documentation identifies the existing tables and modules that own
  current memory truth.
- Public surfaces expose staleness, temporal, injection, and usage feedback
  state without silent fallback. This includes current-state `current`,
  `conflicts`, and `history` results, not only search/list/detail memory items.
- Doctor or an equivalent structured status surface reports:
  - context injection item counts by status, channel, and drop reason;
  - citation parse/match/usage rates;
  - temporal fact total and retrieval-eligible counts;
  - source-anchor state distribution and error count;
  - declared-but-empty production surfaces.
- Deterministic eval gates cover current-state, temporal fact, staleness,
  injection audit, and usage feedback contracts.
- Usage ranking remains default-off until a committed eval report justifies
  changing the default.
- No new parallel schema, IPC layer, or runtime boundary is introduced without
  a follow-up spec that proves the existing contract cannot satisfy the need.

## Open Product Decisions

- Whether the structured observability endpoint should be a new
  `/api/v1/observability` endpoint or a versioned expansion of
  `/api/v1/status`.
- Whether usage feedback should stay purely in doctor/eval reports or also be
  rendered in the default context preamble.
- What minimum eval delta is required before enabling non-zero usage ranking by
  default.
- Whether `verify-before-trust` memories should always be injected with a
  warning, demoted below a threshold, or excluded from some host contexts.
