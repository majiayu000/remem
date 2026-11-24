# Retrieval Router v1 — Product Spec

Refs #934.

## Status

Partial implementation. PR #940 landed the deterministic task-intent planner
and `context-plan` debug surface. PR #1006 (`d458bfeb`) subsequently unified
Context Bundle on `RetrievalPlan` and added explicit SessionStart planning/DB
execution. PR #1020 (`10b2d38d`, implementation commit `477ca5db`) later added
the optional routed MCP `search` projection onto existing production loaders.
That projection was retired on 2026-11-24 (see below). GitHub issue #934 is
closed as not planned. Landed slices stay landed; the original
acceptance still lacks the complete plan-controlled per-channel executor and
full execution-evidence propagation, per-intent goldens, static-vs-router
ablation/default decision, and a closure audit. Remaining work can stay parked
until prioritized; it must not be tracked as overall completion.

Until the registered ablation/default gate lands and passes, static retrieval
remains the product default.

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
- (retired 2026-11-24) MCP `search` used to accept optional task-aware
  routing fields and return a compact `retrieval_plan` audit; see the
  retirement section below;
- explicit `SessionStart` routing is reserved for host lifecycle callers and
  Context Bundle compilation; keyword fallback never infers it from ordinary
  task text;
- stable plan hash (canonical-JSON SHA-256, same convention as #932).

Plan compilation adds no LLM or network call.

## Retirement of the routed MCP search projection (2026-11-24)

Issue #934 was closed as not planned on 2026-11-24 with option (b) of the
2026-11-17 status check: retire the routed MCP `search` projection through the
GH969 retirement ledger and keep the planner and `context-plan`. The
exploratory 2026-11-17 probe found the same `evidence_recall@5` (0.525) for
every arm, and keyword intent resolution lowered P@5 from 0.492 to 0.184.

- Removed: the MCP `search` input fields `task_intent`, `role`, `risk`,
  `token_budget` and `include_superseded`, and the `retrieval_plan` block in
  the `search` response. Their v0.7.0 published identities, plus the v0.7.0
  `search` tool identity whose schema changed, are appended to
  `retired_surfaces` in `docs/specs/GH969/surface-manifest.json`.
- Fingerprint-only entries: the REST `GET`/`HEAD /api/v1/search` v0.7.0
  identities are also in the ledger. The REST request and response did not
  change; the lifecycle REST fingerprint indexes serde declarations by type
  name, and the MCP and REST `SearchParams` share that name, so editing the
  MCP struct changes the REST identity. The new identities are `staged`.
- Kept: `RetrievalPlan`, the planner, `remem context-plan`, Context Bundle
  and SessionStart planning, and the published Rust exports (including
  `memory::service::SearchRoutingPolicy`), which follow their own lifecycle
  rows.
- Compatibility: `search` still ignores input keys it does not declare, so a
  caller that keeps sending the removed fields gets ordinary search and no
  `retrieval_plan`. No stored data or migration is involved.

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
