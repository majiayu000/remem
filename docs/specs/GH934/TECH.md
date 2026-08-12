# Retrieval Router — Tech Spec

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
src/mcp/server/search_tools.rs    MCP search optional router consumer
src/memory/service/search.rs      search execution-policy adapter
```

## Contract

- `RETRIEVAL_PLAN_SCHEMA_VERSION = 1`.
- `RETRIEVAL_ROUTER_POLICY_VERSION = "retrieval_router_v2"` after the
  post-merge caller-scope correction: intent resolution must never widen
  `include_superseded`. The schema remains v1 because the serialized shape is
  unchanged; the policy version distinguishes plans compiled before and after
  this behavioral correction.
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

## MCP search wiring

The first production consumer is MCP `search`. Legacy requests omit the routing
fields and keep the static search policy. A routed request must provide a
non-empty `query` and explicit `project`; it may also provide:

`task_intent`, `role`, `risk`, `token_budget`, `include_superseded`.

The handler compiles a `RetrievalPlan` using the query text as the task. The
search service then applies a bounded execution projection:

- channel weights map to the existing FTS, vector, entity, graph traversal,
  temporal/fact, LIKE fallback, and usage controls;
- `graph_expansion` enables multi-hop search only when the caller did not
  explicitly set `multi_hop=false`, and explain requests keep graph expansion
  disabled because existing multi-hop explain is unsupported;
- `rerank_policy.enabled` gates whether the post-fusion rerank stage is
  requested, and its candidate pool / output k override the ambient reranker
  bounds when the local reranker is enabled;
- high-risk `on_low_evidence` abstention disables raw-archive fallback so
  uncurated chat rows cannot satisfy a high-risk request;
- the response includes `retrieval_plan` metadata with `plan_hash`,
  policy version, intent source, filters, reason codes, selected/disabled
  channels, and applied execution effects.

The current projection deliberately does not invent new database loaders for
decision/session/preference-specific evidence channels; those remain explicit
follow-ups before the router can become default.

## Follow-ups (tracked on #934)

Full per-channel execution into ContextBundle, generated-enrichment execution,
per-intent golden fixtures, static-vs-router ablation and default-on gates.
SessionStart plan hashes already flow through persisted `ContextAudit` rows
into verified coding-bench remem artifacts; that landed slice does not replace
the pending per-intent execution and ablation work.
