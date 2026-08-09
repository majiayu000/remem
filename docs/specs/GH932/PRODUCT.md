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
  `RetrievalPlan`, `ContextBundle`, `ContextAudit`, `ContextItem`.
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
- A DB-backed compiler loads canonical SessionStart candidates and fails
  closed to a `blocked` bundle when canonical loading is incomplete.
- On the healthy production SessionStart path, the renderer consumes the
  bundle's scope, trust, attribution, and relevance decisions from the same
  loaded snapshot. The established section renderer still owns exact
  item/character boundaries, then seals the bundle to the identities that
  survived. Host-visible output remains byte-identical to the legacy path.

## v1 Non-Goals (deferred follow-up work)

- No MCP or REST endpoint. The contract is an internal Rust API only.
- No rerank, graph expansion, or LLM calls in plan or execute.
- No change to the existing SessionStart rendered output or gating.
- No MCP/REST bundle consumer, doctor plan summary, or benchmark artifact;
  those remain follow-up items on #932.
- Load-error fail-open rendering remains on the compatibility path so existing
  user-visible diagnostics are not hidden behind a second failure. Healthy
  loads use the bundle path; the standalone DB compiler still represents an
  incomplete canonical load as a blocked bundle.
- v1 API is experimental; the version fields exist so later revisions can
  break the shape explicitly.

## Success Criteria

- Same request + policy produce byte-identical plan JSON and plan hash.
- Bundles never exceed the total token budget or per-section budgets.
- Every dropped candidate has a machine-readable reason in the audit.
- Degraded modes are explicit and appear in the audit.
- Legacy and bundle-backed SessionStart renders are byte-identical for the
  same snapshot and effective policy.
- `REMEM_CONTEXT_BUNDLE_RENDER_MODE=legacy` provides an explicit rollback to
  the compatibility relevance path; unset or `bundle` uses the bundle.
