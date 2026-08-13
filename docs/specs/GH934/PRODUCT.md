# Retrieval Router v1 — Product Spec

Refs #934.

## Status

Partial implementation. PR #940 landed the deterministic task-intent planner
and `context-plan` debug surface. PR #1006 (`d458bfeb`) subsequently unified
Context Bundle on `RetrievalPlan` and added explicit SessionStart planning/DB
execution. PR #1020 (`10b2d38d`, implementation commit `477ca5db`) later added
the optional routed MCP `search` projection onto existing production loaders.
GitHub issue #934 is currently closed, but its original acceptance still lacks
the complete plan-controlled per-channel executor and full execution-evidence
propagation, per-intent goldens, static-vs-router ablation/default decision,
and a closure audit. The closed remote state therefore must not be read as
proof that the acceptance contract below is complete.

Remaining work requires either reopening #934 or a separately linked
implementation issue before it starts. Until the registered ablation/default
gate lands and passes, static retrieval remains the product default.

## Problem

remem's channels (FTS, vector, entity, temporal, graph, enrichment) keep
growing. Before the optional routed MCP projection, every query used the same
static channel + fusion policy; legacy/default requests still do. Different
tasks need different evidence: "why not async-trait"
wants decisions, superseded history, and git evidence rather than pure
semantic neighbors; a high-risk code change must not share freshness /
trust / abstention policy with casual history browsing.

## Landed Phase A slice

A deterministic, auditable Retrieval Router that compiles a
`ContextRequest` plus an optional explicit intent into a versioned
`RetrievalPlan`:

- six routable intents: `resume_work`, `explain_decision`,
  `debug_failure`, `apply_preference`, `review_change`,
  `explore_history`, each with a test-locked priority-channel mapping;
- per-channel plans (enabled, candidate limit, fusion weight, required
  trust, allowed validity, max contribution, timeout, degradation);
- role / risk / scope / token budget enter the plan; high risk tightens
  trust, disables generated enrichment, requires canonical evidence at
  top 1, and abstains on low evidence;
- intent resolution: explicit caller intent always wins; simple keyword
  rules are the only fallback; unclassifiable tasks conservatively fall
  back to `explore_history` (generic policy);
- `remem context-plan --task ... --json` debug command printing resolved
  intent, enabled/disabled channels, filters, budgets, policy version,
  and reason codes — never memory contents;
- MCP `search` accepts optional task-aware routing fields (`task_intent`,
  `role`, `risk`, `token_budget`, `include_superseded`). When present, it
  compiles the same `RetrievalPlan`, applies the plan to production search
  weights / graph expansion / rerank participation / raw-fallback abstention,
  and returns a compact plan audit (`plan_hash`, reason codes, selected and
  disabled channels) alongside the legacy compact search envelope;
- explicit `SessionStart` routing is reserved for host lifecycle callers and
  Context Bundle compilation; keyword fallback never infers it from ordinary
  task text;
- stable plan hash (canonical-JSON SHA-256, same convention as #932).

Plan compilation adds no LLM or network call. Routed MCP search preserves the
existing search path's optional embedding and rerank provider behavior.

## Boundaries

- #932 Context Bundle owns the request/bundle contract; the router
  reuses its `ContextRequest`, filters, trust/validity types.
- #851 owns the rerank model and execution; the router only selects
  participation, candidate pool / output k, and timeout fallback.
- #853 graph expansion and #850/#928 enrichment workers stay separate;
  `generated_enrichment` is a capped, attributed, skippable signal here.
- #854 SessionStart budget gate is untouched; `SessionStart` is an explicit
  lifecycle intent for the bundle/compiler path and is never inferred from
  task text.

## Completion work not yet landed

The complete plan-controlled per-channel executor, propagation of its
execution evidence through ContextBundle/ContextAudit, per-intent golden
fixtures, static-vs-router ablation, benchmark evidence, and an explicit
default decision remain pending. Existing SessionStart planning and the
bounded MCP search projection do not satisfy those acceptance items.
