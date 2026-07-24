# Context Bundle v1 — Tech Spec

Refs #932.

## Module Layout

```
src/context_bundle.rs      module root, public re-exports
src/context_bundle/
  domain.rs                versioned serde DTOs
  planner.rs               deterministic plan(request) + plan hash
  executor.rs              execute(plan, inputs) -> ContextBundle
  policy.rs                policy version, budgets, validation, reasons
  audit.rs                 audit entry construction
  tests/                   determinism, budgets, schema snapshots
```

## Contract

- `CONTEXT_BUNDLE_SCHEMA_VERSION = 1` on request, plan, bundle, and audit.
- `CONTEXT_BUNDLE_POLICY_VERSION = "context_bundle_v1"`.
- All DTOs derive `Serialize`/`Deserialize` with `snake_case` enum renames.
- `ContextRequest`: task, project ref, branch, worktree, role, as_of_epoch,
  token_budget, risk class, include_superseded.
- `ContextPlan`: intent, relevance query/k, planned channels, filters,
  named per-section token budgets, policy version, plan hash.
- `ContextBundle`: per-section `ContextItem` lists (preferences,
  failure_lessons, current_truth, memory_index, recent_sessions,
  workstreams), degraded mode, audit.
- `ContextItem` carries `source_kind` (canonical / generated /
  graph_derived), `canonical_ref`, `projection_ref`, `evidence_refs`,
  `validity`, and `trust` so derived projections can never masquerade as
  canonical memory.

## Planner

- `plan(&ContextRequest) -> Result<ContextPlan>`; invalid requests (empty
  project key, zero token budget, schema mismatch) are rejected.
- Budgets derive from `crate::context::ContextLimits::default()` converted
  from char limits to token estimates (4 chars per token). Env-var limit
  overrides are intentionally not read in v1 so the plan is a pure function
  of the request plus the compiled policy version.
- `plan_hash` = SHA-256 over the canonical serde JSON of the plan with an
  empty `plan_hash` field. No timestamps or randomness are hashed.

## Executor

- `execute(&ContextPlan, &ExecutorInputs) -> ContextBundle` over
  caller-provided candidates; v1 has no DB access.
- Re-validates the plan (schema/policy version, filters); a failed
  validation yields a `blocked` bundle whose audit drops every candidate.
- Scope checks run in both layers: the planner validates the request
  scope, the executor re-checks item project/branch against plan filters.
- Relevance governance reuses `build_sessionstart_relevance_plan` for the
  lessons / memory_index / sessions channels — the same policy
  (`sessionstart_significant_token_v1`) and drop reasons as SessionStart.
- Budget enforcement order: per-channel item limit, per-channel token
  budget, then total token budget over a fixed section order; each drop is
  recorded with a machine-readable reason.
- `enrichment_available = false` degrades to `canonical_only`: generated /
  graph-derived candidates drop with `canonical_only_degraded`.

## Reuse, Not Rewrite

`src/context/policy.rs` (`ContextLimits`) and `src/context/relevance.rs`
(`build_sessionstart_relevance_plan`, `RelevanceCandidate`,
`RelevanceSection`, `SESSIONSTART_RELEVANCE_POLICY_VERSION`) are widened
from `pub(super)` to `pub(crate)` and re-exported from `src/context.rs`.
No SessionStart rendering behavior changes.

## Tests

- Plan determinism: repeated planning yields identical JSON and hash;
  different requests yield different hashes.
- Budget enforcement: channel and total token budgets, item limits.
- Scope/trust/validity drops with exact reasons.
- Degraded modes: `canonical_only` and `blocked`.
- Schema snapshots: serialized plan and bundle JSON compared against
  fixed `serde_json::json!` literals.
