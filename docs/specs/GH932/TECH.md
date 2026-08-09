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
  are skipped rather than contacted.
- Rows removed by the poisoning gate remain in `ContextAudit` with
  `reason=poisoning_gate`, stable identity, channel, source, and validity only;
  their title and text are cleared before executor input and cannot reach the
  returned JSON.

## Tests

- Plan determinism: repeated planning yields identical JSON and hash;
  different requests yield different hashes.
- Budget enforcement: channel and total token budgets over title plus text,
  item limits.
- Scope/trust/validity drops with exact reasons.
- High-risk trust floor and low-evidence abstention, persisted user-authored
  trust mapping, and redacted poisoning-gate audit coverage.
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
