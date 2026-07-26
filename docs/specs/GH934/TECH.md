# Retrieval Router v1 — Tech Spec

Refs #934.

## Module Layout

```
src/retrieval_router.rs    module root, public re-exports
src/retrieval_router/
  domain.rs                versioned RetrievalPlan / ChannelPlan / policy DTOs
  intent.rs                deterministic intent resolution
  planner.rs               per-intent mapping tables, adjustments, plan hash
  tests.rs                 mapping locks, determinism, policy, validation
src/cli/context_types.rs   clap args (ContextPlanArgs, ContextGateAction)
src/cli/actions/context_plan.rs   `remem context-plan` handler
```

## Contract

- `RETRIEVAL_PLAN_SCHEMA_VERSION = 1`,
  `RETRIEVAL_ROUTER_POLICY_VERSION = "retrieval_router_v1"`.
- `ContextIntent` (shared with #932 in `context_bundle::domain`) gains
  the six router variants; `SessionStart` stays the bundle-planner
  intent and is not routable — explicit `SessionStart` falls back to
  `explore_history` with reason `session_start_not_routable`.
- `RetrievalPlan`: schema/policy versions, intent + intent_source, role,
  risk, reason_codes, `channel_plans` (one entry per known channel,
  enabled or disabled, in `RetrievalChannel::ORDERED` order), reused
  `ContextFilters`, `RerankPolicy`, `TrustPolicy`, `FreshnessPolicy`,
  `AbstentionPolicy` (v1 placeholders), token_budget, plan_hash.
- `ChannelPlan`: channel, enabled, candidate_limit, weight,
  required_trust, allowed_validity, max_contribution, timeout_ms,
  degradation (`skip_channel` | `fail_closed`; only `canonical_fts` is
  fail-closed base evidence).
- 15 channels: `canonical_fts`, `canonical_vector`,
  `generated_enrichment`, `entity_graph`, `graph_expansion`, `temporal`,
  `workstreams`, `session_outcomes`, `decisions`, `superseded_history`,
  `git_evidence`, `benchmark_evidence`, `failure_lessons`,
  `preferences`, `constraints`.

## Planner

- `plan(&ContextRequest, Option<ContextIntent>) -> Result<RetrievalPlan>`
  is a pure function of its inputs plus the compiled policy: no clock,
  env, randomness, LLM, or network. Request validation reuses
  `context_bundle::validate_request`.
- Every intent gets the baseline `canonical_fts` (0.8) /
  `canonical_vector` (0.7) / `generated_enrichment` (0.3, max
  contribution 2, skip-degradation) plus its priority channels from the
  issue's per-intent lists; the mapping table is locked by unit tests.
- Rerank participation (#851 boundary): enabled only for
  `explain_decision`, `debug_failure`, `review_change` with pool 50 /
  k 10 and `skip_rerank` timeout fallback.
- High risk: trust floor `trusted`, enrichment channel disabled,
  `require_canonical_evidence_top1`, abstention `on_low_evidence`; each
  adjustment appends a machine-readable reason code. Reviewer role
  force-enables the `constraints` channel.
- `plan_hash` = SHA-256 over canonical serde JSON with the hash field
  empty — the same convention as `ContextPlan.plan_hash` (#932).

## Intent resolution

Explicit intent > keyword fallback > `explore_history` default.
Keyword rules run in a fixed priority order (debug_failure,
explain_decision, review_change, resume_work, apply_preference) as
case-insensitive substring checks; rules can only pick an intent, never
widen scope, lower trust, or bypass abstention.

## CLI

`remem context-plan --task <text> [--intent ...] [--project/--cwd]
[--branch] [--role] [--risk] [--token-budget] [--include-superseded]
[--as-of-epoch] [--json]` — prints the plan summary or full JSON.
`--as-of-epoch` defaults to 0 (no pin) so default output is fully
deterministic. The command never opens the database and never prints
memory contents.

## Follow-ups (tracked on #934)

Execution wiring (plan -> retrieve/rerank -> ContextBundle), plan hash
into `ContextAudit` and benchmark artifacts at execution time,
per-intent golden fixtures, static-vs-router ablation and default-on
gates.
