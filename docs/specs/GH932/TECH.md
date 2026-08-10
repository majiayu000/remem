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
  persistence.rs           canonical audit hashing, append-only store/verify
  tests/                   determinism, budgets, schema snapshots
src/retrieval_router/
  planner.rs               deterministic unified RetrievalPlan + plan hash
src/context/
  bundle_candidates.rs     same-snapshot SessionStart candidate adapter
  summary_query.rs         pre-query poisoning + summary preselection snapshot
  render.rs                production consumer and exact bundle sealing
src/doctor/
  context_compiler.rs      payload-free compiler capability/degraded check
src/mcp/
  types.rs                 experimental v1 request DTO
  server/context_tools.rs  DB-backed context_bundle tool
  server/tool_contracts/   closed output schema + wire compatibility tests
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
  `plan_session_start_with_limits`; a SHA-256 fingerprint of the complete
  limits object is carried in `reason_codes`, so loader-only limits such as
  candidate fetch caps also affect the final plan hash. The compiler passes
  the same resolved limits to the loader and never re-reads the environment.
- SessionStart plans also carry a SHA-256 fingerprint of effective project,
  branch, and worktree scope in `reason_codes`, so worktree-specific preference
  loading cannot reuse an indistinguishable plan hash.
- MCP Context Bundle plans additionally carry a SHA-256 fingerprint of the
  effective local-only embedding execution profile: policy version, execution
  mode, resolved provider, model identity (including the verified local model
  artifact digest), and dimensions. Switching among local, feature-hash,
  skipped remote/off, or blocked profiles therefore changes `plan_hash` before
  candidate loading without exposing configuration or model paths.
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
- Trust floors and abstention are executable policy, not plan-only metadata.
  The DB adapter maps `source_trust_class=user_prompt` to `trusted`; high-risk
  plans drop standard rows and return an audited low-evidence abstention when
  the minimum selected count is not met.
- Relevance governance reuses `build_sessionstart_relevance_plan` for the
  lessons / memory_index / sessions channels — the same policy
  (`sessionstart_significant_token_v1`) and drop reasons as SessionStart.
- Strict execution orders relevance-governed survivors by the selector's
  ranked stable keys before section limits consume them. Canonical loader order
  remains unchanged for non-governed channels and renderer-deferred execution.
- Budget enforcement counts the complete returned item (`title` + `text`). Its
  order is per-channel item limit, per-channel token
  budget, then total token budget over a fixed section order; each drop is
  recorded with a machine-readable reason.
- The production renderer uses `BudgetEnforcement::DeferToRenderer`: scope,
  trust, attribution, and relevance remain final in the bundle, while the
  existing renderer applies exact item and character limits. It must then call
  `seal_session_start_bundle`, which prunes bundle sections and updates audit
  reasons/counts to the exact surviving stable identities.
- `enrichment_available = false` degrades to `canonical_only`: generated /
  graph-derived candidates drop with `canonical_only_degraded`.
- The MCP/local-only adapter uses `SearchWeights::context_bundle_v1()` and
  ignores `REMEM_USAGE_WEIGHT`; changing those fixed weights requires a router
  policy version bump. A missing key for a configured remote embedding provider
  skips the vector channel rather than blocking usable lexical channels. The
  same local-only provider resolution feeds both execution and the embedding
  profile fingerprint bound into the plan hash.

## Reuse, Not Rewrite

`src/context/policy.rs` (`ContextLimits`) and `src/context/relevance.rs`
(`build_sessionstart_relevance_plan`, `RelevanceCandidate`,
`RelevanceSection`, `SESSIONSTART_RELEVANCE_POLICY_VERSION`) are shared by the
unified router and executor. Candidate adaptation reuses the exact preference
and Core identities chosen by the compatibility renderer, and unselected
indexed Core-type memories are routed to `memory_index`. No SessionStart
output bytes or gate behavior change. `REMEM_CONTEXT_BUNDLE_RENDER_MODE=legacy`
is the explicit rollback.

## Durable SessionStart Audit

- Migration v081 creates `context_bundle_audits`, keyed uniquely by
  `injection_run_id`. A trigger requires at least one matching
  `context_injection_items` row before insertion, so the bundle summary cannot
  become detached from the established item-level emission audit. The migration
  also indexes `context_injection_items(injection_run_id)`, which is the join
  and integrity-check key used by every persisted audit.
- The row stores `bundle_schema_version`, `plan_schema_version`,
  `policy_version`, `relevance_policy_version`, `plan_hash`, `audit_hash`,
  `degraded_mode`, candidates/selected/dropped counts, token budget/estimate,
  truncation reason, canonical `audit_json`, and creation epoch. It stores no
  bundle section, memory title, memory text, or rendered hook output.
- `audit_json` is a recursively key-sorted canonical serialization of the
  already-redacted `ContextAudit`. `audit_hash` is lowercase SHA-256 hex over
  a recursively key-sorted envelope containing that audit value and the stored
  `plan_schema_version`, so changing the version cannot preserve the hash.
  Array order remains the deterministic audit-entry order established by the
  executor/sealer.
- SessionStart writes finalized `context_injection_items` and the bundle audit
  in one SQLite transaction. Each emission gets a 128-bit OS-generated
  nonce in its `injection_run_id`, so distinct same-second PromptSubmit and
  SessionStart invocations cannot collapse onto one item set. A retry for an
  explicit existing run succeeds only when the stored and incoming hashes
  match and matching item rows still exist; otherwise it returns an integrity
  error. The table rejects in-place updates; retention cleanup may delete
  expired rows.
- The production delta gate receives the complete renderer item-end map,
  including preferences, and rewinds an over-limit preview to the last complete
  item. Persistence then clones and reseals the bundle to those finalized
  identities, marks later selected entries as `delta_preview`, recomputes
  selection counts and emitted-output token estimate, and hashes that post-gate
  audit so item rows and the verified bundle audit describe the same
  SessionStart bytes. Output-only delta truncation is recorded even when every
  selected identity survives. If writing the gate state fails after a delta is
  built, the fail-open decision preserves that delta's item boundary and
  truncation metadata so subsequent audit persistence still describes the
  emitted preview rather than the pre-gate bundle. Full, bypassed, and full
  fail-open emissions also clone and reseal the rendered bundle after final
  debug and hook-integrity annotations are appended, so their persisted token
  estimate describes the complete emitted output rather than the pre-annotation
  render.
- Verified reads validate the stored `plan_schema_version` before parsing or
  decoding `audit_json`, canonicalize and re-hash the version-bound envelope,
  dispatch to that version's decoder, then compare every
  denormalized contract field with the parsed audit. Supported historical
  versions remain readable after the current planner advances; unknown stored
  versions fail explicitly. A hash mismatch, malformed JSON, missing item link,
  or summary mismatch is a hard error.
- `REMEM_CONTEXT_GATE_RETENTION_DAYS` also bounds persisted bundle audits.
  Cleanup runs exactly once after a context invocation successfully opens the
  database and before pre-render gating, so strict pre-render suppressions,
  gate-off, and non-gated direct emissions cannot accumulate unbounded rows. It
  deletes only rows older than the same cutoff and never rewrites an audit.
  Item-level retention remains governed by its existing consumers.
- Audit persistence is attempted only for emitted Bundle-backed SessionStart
  output. Legacy rollback and pre-render suppressed invocations do not invent
  a plan/audit. Any write failure is logged at error level while preserving the
  existing hook fail-open output contract; once an emission run ID has been
  generated, the error chain and diagnostic include that attempted ID.

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

## Experimental MCP Consumer

- `context_bundle` requires `schema_version=1` and a non-blank task. Project
  scope is explicit or derived from `cwd`; the loader uses `worktree`, then
  `cwd`, then the MCP server process working directory.
- Role/risk default to `coder` / `medium`, token budget defaults to 4000, and
  invalid enum values, schema versions, blank scope values, or zero budget are
  MCP `invalid_request` errors. Enrichment is reported available because this
  v1 adapter loads canonical SessionStart candidates only.
- Nonzero `as_of_epoch` and `include_superseded=true` are rejected explicitly:
  the production SessionStart loader does not reconstruct historical rows or
  load superseded rows yet, so accepting those values would return silently
  incorrect context. The fields remain in the versioned request for a later
  canonical-loader phase.
- The handler calls `compile_session_start_bundle`; canonical partial-load
  failures therefore return a versioned `blocked` bundle with an audit instead
  of masquerading as an empty successful project.
- The tool publishes a closed typed `outputSchema`. Success keeps the historical
  single JSON text content and mirrors the same object into
  `structuredContent`, so clients that do not consume structured MCP output
  continue to work. Nullable v1 fields remain required on the wire and must be
  serialized explicitly as a value or `null`.
- Candidate poisoning checks may quarantine unsafe persisted rows, matching the
  production SessionStart compiler. Tool annotations therefore do not claim
  read-only or idempotent behavior even though the compilation path performs no
  foreground LLM or network call. Query embeddings on this endpoint are
  restricted to local/local-feature-hash providers; configured API providers
  are skipped rather than contacted, while their resolved local fallback
  remains available. The loader does not invoke the shared SessionStart
  reranker because rerank configuration and drops are not represented in the
  v1 retrieval plan or audit.
- Rows removed by the poisoning gate remain in `ContextAudit` with
  `reason=poisoning_gate`, stable identity, channel, source, and validity only;
  their title and text are cleared before executor input and cannot reach the
  returned JSON. Summary and workstream gates run before implicit retrieval
  query construction, so rejected payloads cannot influence memory recall.
- Canonical memory and session selectors carry fetched-but-omitted rows from
  cluster/session deduplication, self-diagnostic limits, stale fallback, and
  item limits on the loaded snapshot. The preference selector does the same
  for `claude_md_dedup`, `preference_similarity_dedup`,
  `project_topic_override`, and `preference_char_limit`. Both DB-backed MCP and
  production-renderer compilers pass these as audit-only preselection drops.

## Tests

- Plan determinism: repeated planning yields identical JSON and hash;
  different requests yield different hashes.
- Budget enforcement: channel and total token budgets over title plus text,
  item limits.
- Scope/trust/validity drops with exact reasons.
- High-risk trust floor and low-evidence abstention, persisted user-authored
  trust mapping, redacted poisoning-gate audit coverage, and canonical
  memory/session/preference preselection-drop coverage.
- Degraded modes: `canonical_only` and `blocked`.
- Schema snapshots: serialized plan and bundle JSON compared against
  fixed `serde_json::json!` literals.
- Production parity: the legacy and bundle-backed render paths consume one
  cloned snapshot and must emit byte-identical output.
- Deferred-budget sealing: executor candidates survive approximate token
  budgets until the exact renderer seals the bundle.
- Doctor capability: bundle / legacy / invalid render-mode states, plus full
  doctor human and JSON inclusion without memory payload text.
- MCP input schema snapshot: exact v1 properties, required fields, and closed
  object behavior; frozen minimal-v1 request compatibility and rejection of
  unsupported schema versions.
- MCP output contract: typed nested bundle/audit schemas, real served-wire
  success validation, and exact legacy-text / `structuredContent` parity.
- MCP execution isolation: a configured remote embedding provider is never
  contacted, its resolved local fallback remains usable, and ambient invalid
  or enabled rerank configuration is not evaluated by the bundle loader. The
  local-only embedding profile fingerprint changes when effective vector
  provider/model/dimensions or skipped/blocked mode changes.
- Persistence: migration/schema-drift coverage, atomic item/audit writes,
  same-run retry idempotency, conflicting-retry and stored-row tamper
  detection, retention cleanup, and proof that memory title/body/rendered
  payloads never enter `context_bundle_audits`.
