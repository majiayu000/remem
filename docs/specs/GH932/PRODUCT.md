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
  plan and the same hash. Effective SessionStart loader limits are fingerprinted
  into the plan before hashing; no timestamps or randomness enter the hash.
- `ContextAudit` records every candidate considered, selected, and dropped
  with a machine-readable reason, plus token estimate, policy version, and
  degraded mode (`full` / `canonical_only` / `blocked`).
- Poisoning-gate drops retain only stable identity and attribution in the
  audit; unsafe title/text payloads never enter bundle sections or wire JSON.
- Session summaries and workstreams pass the poisoning gate before they can
  contribute text to the implicit retrieval query.
- Memories and session summaries fetched but omitted by canonical clustering,
  self-diagnostic, stale-fallback, session-identity, or item-limit selectors,
  plus preferences omitted by CLAUDE.md deduplication, similarity
  deduplication, scope override, or character limits, retain their stable
  identity and exact selection reason in the audit.
- Schema snapshot tests pin the serialized JSON structure.
- A DB-backed compiler loads canonical SessionStart candidates and fails
  closed to a `blocked` bundle when canonical loading is incomplete.
- On the healthy production SessionStart path, the renderer consumes the
  bundle's scope, trust, attribution, and relevance decisions from the same
  loaded snapshot. The established section renderer still owns exact
  item/character boundaries, then seals the bundle to the identities that
  survived. Host-visible output remains byte-identical to the legacy path.
- `remem doctor` includes a payload-free `Context compiler` capability check.
  It reports the production render mode, expected degraded state, bundle and
  plan schema versions, router/relevance policy versions, and points operators
  to `remem context-plan` for a request-specific plan summary. Explicit legacy
  rollback is visible as a warning; an invalid render-mode value fails loudly.
- An experimental MCP `context_bundle` tool accepts the versioned v1 request
  fields and returns the complete `ContextBundle` JSON contract. It reuses the
  DB-backed SessionStart compiler, performs no foreground LLM or network call,
  disables remote query embeddings even when an API provider is configured,
  permits the resolved local fallback for that provider, disables ambient
  reranking because it is absent from the v1 plan and plan hash, fixes hybrid
  retrieval weights to the bundle v1 policy instead of reading ambient
  operator overrides,
  and publishes a closed MCP output schema while preserving the same JSON in
  the legacy text content field for older MCP clients.

## v1 Non-Goals (deferred follow-up work)

- No REST endpoint. The first external consumer is the experimental MCP tool;
  general REST exposure and a stable public API commitment remain deferred.
- No rerank, graph expansion, or LLM calls in plan or execute. The MCP loader
  also disables a globally configured reranker rather than applying an
  unplanned top-k cut.
- No change to the existing SessionStart rendered output or gating.
- No benchmark artifact yet; plan/audit hash persistence remains a follow-up
  item on #932.
- The doctor capability check does not expose memory payloads or persist
  per-session audit entries. Durable audit history remains a later phase.
- Load-error fail-open rendering remains on the compatibility path so existing
  user-visible diagnostics are not hidden behind a second failure. Healthy
  loads use the bundle path; the standalone DB compiler still represents an
  incomplete canonical load as a blocked bundle.
- v1 API is experimental; the version fields exist so later revisions can
  break the shape explicitly.

## Success Criteria

- Same request + policy produce byte-identical plan JSON and plan hash.
- Bundles never exceed the total token budget or per-section budgets; estimates
  include both item title and text.
- Every dropped candidate has a machine-readable reason in the audit.
- High-risk plans enforce the trusted-only floor and low-evidence abstention;
  direct user-authored (`user_prompt`) memories are the trusted v1 class.
- Degraded modes are explicit and appear in the audit.
- Legacy and bundle-backed SessionStart renders are byte-identical for the
  same snapshot and effective policy.
- `REMEM_CONTEXT_BUNDLE_RENDER_MODE=legacy` provides an explicit rollback to
  the compatibility relevance path; unset or `bundle` uses the bundle.
- `remem doctor` reports bundle mode as healthy, legacy rollback as degraded,
  and invalid configuration as failed, without loading or printing memories.
- MCP `context_bundle` validates request schema/role/risk/budget, returns the v1
  bundle shape through both legacy text and `structuredContent`, and has frozen
  input/output compatibility tests.
