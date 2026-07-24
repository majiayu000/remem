# Context Bundle v1 — Product Spec

Refs #932.

## Problem

Callers of remem today face "several search tools plus one pre-rendered text
blob". As retrieval enrichment, rerank, graph expansion, and host-native
memory land, callers should not need to understand every retrieval detail.
remem should act as a policy-aware context compiler: take a task request,
produce an explainable retrieval plan, and return a budgeted, auditable
context bundle.

## v1 Scope

- A versioned, serde-JSON internal Rust contract: `ContextRequest`,
  `ContextPlan`, `ContextBundle`, `ContextAudit`, `ContextItem`.
- A deterministic planner with `plan(request)` / `execute(plan, inputs)`
  separation. The v1 planner wraps the existing SessionStart selection
  policy (section limits from `ContextLimits`, relevance selection from
  `build_sessionstart_relevance_plan`); it does not rewrite any retrieval
  channel.
- Stable plan hash: the same request and policy always produce the same
  plan and the same hash. No timestamps or randomness enter the hash.
- `ContextAudit` records every candidate considered, selected, and dropped
  with a machine-readable reason, plus token estimate, policy version, and
  degraded mode (`full` / `canonical_only` / `blocked`).
- Schema snapshot tests pin the serialized JSON structure.

## v1 Non-Goals (deferred follow-up work)

- No MCP or REST endpoint. The contract is an internal Rust API only.
- No rerank, graph expansion, or LLM calls in plan or execute.
- No change to the existing SessionStart rendered output or gating.
- No DB-backed executor wiring; v1 executes over caller-provided candidate
  items. Wiring the executor to `load_context_render_inputs`, the bundle
  renderer parity path, doctor plan summaries, and benchmark artifacts are
  follow-up items on #932.
- v1 API is experimental; the version fields exist so later revisions can
  break the shape explicitly.

## Success Criteria

- Same request + policy produce byte-identical plan JSON and plan hash.
- Bundles never exceed the total token budget or per-section budgets.
- Every dropped candidate has a machine-readable reason in the audit.
- Degraded modes are explicit and appear in the audit.
