# Retrieval Router v1 — Product Spec

Refs #934.

## Problem

remem's channels (FTS, vector, entity, temporal, graph, enrichment)
keep growing, but every query runs the same static channel + fusion
policy. Different tasks need different evidence: "why not async-trait"
wants decisions, superseded history, and git evidence rather than pure
semantic neighbors; a high-risk code change must not share freshness /
trust / abstention policy with casual history browsing.

## What v1 ships

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
- stable plan hash (canonical-JSON SHA-256, same convention as #932).

No LLM or network call exists on any router path.

## Boundaries

- #932 Context Bundle owns the request/bundle contract; the router
  reuses its `ContextRequest`, filters, trust/validity types.
- #851 owns the rerank model and execution; the router only selects
  participation, candidate pool / output k, and timeout fallback.
- #853 graph expansion and #850/#928 enrichment workers stay separate;
  `generated_enrichment` is a capped, attributed, skippable signal here.
- #854 SessionStart budget gate is untouched; `SessionStart` is not a
  routable intent and maps to the generic policy.

## Remaining out of scope (follow-ups on #934)

Full per-channel evidence loaders, generated-enrichment execution,
per-intent golden fixtures, static-vs-router ablation, and eval gates for
making the router the default.
