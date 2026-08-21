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
  operator overrides, fingerprints the effective local-only embedding mode,
  provider, model artifact identity, and dimensions into the plan hash,
  and publishes a closed MCP output schema while preserving the same JSON in
  the legacy text content field for older MCP clients.
- Every healthy Bundle-backed SessionStart emission persists one append-only
  audit row linked to the existing per-item audit rows by
  `injection_run_id`. The row contains the bundle/plan schema versions,
  router and relevance-policy versions, plan hash, degraded mode, aggregate
  selection and token-budget fields, truncation reason, and a SHA-256 hash of a
  payload-free canonical envelope containing `plan_schema_version` and the
  canonical `ContextAudit`. The canonical audit JSON is retained so benchmark
  verifiers can reconstruct and recompute the envelope hash; it contains only
  stable identities, attribution, scores, reason codes, and counts, never
  memory title or body text. When the context gate emits a delta, the persisted
  preview rewinds to the last complete item boundary and the persisted audit is
  resealed to the identities and token estimate in that emitted delta;
  gate-dropped entries carry the `delta_preview` reason, and output-only
  truncation remains visible even when no candidate is dropped.
- SessionStart item rows and the bundle audit commit atomically. Retrying the
  same `injection_run_id` is idempotent only when the canonical audit hash is
  identical; a conflicting retry or later hash/summary mismatch is reported
  as integrity failure. Persistence failures remain fail-open for hook output
  but are logged at error level with the run identity and cause.

## v1 Non-Goals (deferred follow-up work)

- No REST endpoint. The first external consumer is the experimental MCP tool;
  general REST exposure and a stable public API commitment remain deferred.
- No rerank, graph expansion, or LLM calls in plan or execute. The MCP loader
  also disables a globally configured reranker rather than applying an
  unplanned top-k cut.
- The original GH932 v1 rollout made no change to SessionStart rendered output
  or gating. GH933's bounded v1 consumer, released later in v0.6.81,
  supersedes that historical non-goal only for CurrentTruth Core selection,
  stable evidence/projection references, and explicit conflict abstention. It
  is not permission for unrelated Context Bundle output or gate changes, and
  it does not complete GH933's broader Phase B acceptance.
- Coding-bench remem runs must consume the durable audit for the exact
  SessionStart `injection_run_id` that produced their context. The run artifact
  records the bundle/plan and policy versions, plan/audit hashes, a separate
  injection-run binding hash, degraded mode, candidate/selection/drop counts,
  token budget/estimate, truncation reason, and the payload-free canonical
  audit JSON needed to recompute the hashes.
- A remem run without a verified audit is a runtime contract failure even when
  the coding task succeeds. `no_memory` and curated-file controls mark the
  audit contract `not_applicable` and never carry a remem audit snapshot.
- Report verification canonicalizes the embedded audit JSON, recomputes its
  SHA-256, checks every denormalized field, and rejects a snapshot that differs
  from the persisted injection run loaded during benchmark setup.
- The doctor capability check does not expose memory payloads. Durable audit
  rows are an internal production/benchmark surface, not a new doctor payload.
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
- A persisted audit can be loaded only after its canonical JSON hash and
  denormalized summary fields verify; tampering fails closed. Retention cleanup
  removes expired bundle-audit rows without mutating surviving rows.
