# Context Bundle v1 — Tech Spec

Refs #932.

## Module Layout

```
src/context_bundle.rs      module root, public re-exports
src/context_bundle/
  domain.rs                versioned serde DTOs
  compile.rs               DB and production-renderer compile paths
  executor.rs              execute(plan, inputs) -> ContextBundle
  policy.rs                policy version, budgets, validation, reasons
  audit.rs                 audit entry construction
  tests/                   determinism, budgets, schema snapshots
src/retrieval_router/
  planner.rs               deterministic unified RetrievalPlan + plan hash
src/context/
  bundle_candidates.rs     same-snapshot SessionStart candidate adapter
  render.rs                production consumer and exact bundle sealing
src/doctor/
  context_compiler.rs      payload-free compiler capability/degraded check
```

## Contract

- `CONTEXT_BUNDLE_SCHEMA_VERSION = 1` on request, plan, bundle, and audit.
- The unified plan policy version is `retrieval_router_v2`; bundle and audit
  carry that exact version and plan hash.
- All DTOs derive `Serialize`/`Deserialize` with `snake_case` enum renames.
- `ContextRequest`: task, project ref, branch, worktree, role, as_of_epoch,
  token_budget, risk class, include_superseded.
- `RetrievalPlan`: intent, relevance query/k, planned channels, filters,
  named per-section token budgets, policy version, plan hash.
- `ContextBundle`: per-section `ContextItem` lists (preferences,
  failure_lessons, current_truth, memory_index, recent_sessions,
  workstreams), degraded mode, audit.
- `ContextItem` carries `source_kind` (canonical / generated /
  graph_derived), `canonical_ref`, `projection_ref`, `evidence_refs`,
  `validity`, and `trust` so derived projections can never masquerade as
  canonical memory.

## Planner

- `plan(&ContextRequest) -> Result<RetrievalPlan>`; invalid requests (empty
  project key, zero token budget, schema mismatch) are rejected.
- General plans derive budgets from `ContextLimits::default()`. The production
  SessionStart adapter passes its already-resolved effective limits to
  `plan_session_start_with_limits`; those values are hashed into the plan and
  are never re-read by the executor.
- `plan_hash` = SHA-256 over the canonical serde JSON of the plan with an
  empty `plan_hash` field. No timestamps or randomness are hashed.

## Executor

- `execute(&RetrievalPlan, &ExecutorInputs) -> ContextBundle` remains a pure
  executor over caller-provided candidates. `compile_session_start_bundle`
  supplies a DB-backed adapter, and `compile_session_start_for_renderer`
  consumes the production renderer's already-loaded canonical snapshot.
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
- The production renderer uses `BudgetEnforcement::DeferToRenderer`: scope,
  trust, attribution, and relevance remain final in the bundle, while the
  existing renderer applies exact item and character limits. It must then call
  `seal_session_start_bundle`, which prunes bundle sections and updates audit
  reasons/counts to the exact surviving stable identities.
- `enrichment_available = false` degrades to `canonical_only`: generated /
  graph-derived candidates drop with `canonical_only_degraded`.

## Reuse, Not Rewrite

`src/context/policy.rs` (`ContextLimits`) and `src/context/relevance.rs`
(`build_sessionstart_relevance_plan`, `RelevanceCandidate`,
`RelevanceSection`, `SESSIONSTART_RELEVANCE_POLICY_VERSION`) are shared by the
unified router and executor. Candidate adaptation reuses the exact preference
and Core identities chosen by the compatibility renderer, and unselected
indexed Core-type memories are routed to `memory_index`. No SessionStart
output bytes or gate behavior change. `REMEM_CONTEXT_BUNDLE_RENDER_MODE=legacy`
is the explicit rollback.

## Doctor Capability Check

- `context::context_bundle_render_mode()` is the single parser for
  `REMEM_CONTEXT_BUNDLE_RENDER_MODE`; production rendering and doctor use the
  same result, so diagnostics cannot disagree with runtime behavior.
- `ContextBundleRenderMode::Bundle` reports `Status::Ok` with
  `degraded_mode=full`. `Legacy` reports `Status::Warn` with
  `degraded_mode=legacy_rollback`. Invalid/non-Unicode configuration reports
  `Status::Fail` with the parser error.
- Detail is deliberately payload-free and stable enough for both human and
  `doctor --json` check output: production consumer, render/degraded mode,
  bundle schema, plan schema, router policy, and SessionStart relevance policy.
- Request-specific plan inspection remains `remem context-plan`; doctor names
  that command rather than compiling a synthetic project/task or touching the
  memory database.

## Tests

- Plan determinism: repeated planning yields identical JSON and hash;
  different requests yield different hashes.
- Budget enforcement: channel and total token budgets, item limits.
- Scope/trust/validity drops with exact reasons.
- Degraded modes: `canonical_only` and `blocked`.
- Schema snapshots: serialized plan and bundle JSON compared against
  fixed `serde_json::json!` literals.
- Production parity: the legacy and bundle-backed render paths consume one
  cloned snapshot and must emit byte-identical output.
- Deferred-budget sealing: executor candidates survive approximate token
  budgets until the exact renderer seals the bundle.
- Doctor capability: bundle / legacy / invalid render-mode states, plus full
  doctor human and JSON inclusion without memory payload text.
