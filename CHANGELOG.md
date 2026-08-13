# Changelog

## Unreleased

### Added
- Staged source version `0.6.73` for the retrieval-enrichment P0: schema v083
  defers incomplete upgrade-time history instead of scheduling an AI backfill,
  one-shot workers admit only four potential AI work items across queues and
  stop admitting after 180 seconds, daemon enrichment batches are spaced by 60
  seconds, rows exhaust after three failures, doctor separates
  deferred and exhausted state, and GPT-5.6 Codex credit models no longer use
  the generic GPT-5 static USD estimate.
- Staged source version `0.6.72` for the GH-931 paired-report slice: task-cluster
  statistics are emitted only for a verifier-passing official matrix whose
  tuples carry unique attempt identities and confirmed target-start state.
  Integrity-invalid or pre-target tuples now keep the matrix `insufficient`
  without publishing rates or confidence intervals.
- Staged source version `0.6.70` for GH-981: dynamic MCP output-schema
  extension points now use the object-form unconstrained schema `{}` instead
  of JSON Schema's boolean `true` shorthand, preserving the same open payload
  contract while remaining compatible with Glama's descriptor validator.
- Staged source version `0.6.69` for GH-931: coding-bench condition ids now
  converge on the flagship matrix names. The default dry-run plans
  `no_memory`, `curated_file_budgeted`, and `remem_e2e`; live execution fails
  closed for pending primary adapters while `remem_seeded_sessionstart` and
  `curated_file_expert` remain available through the implemented diagnostic
  matrix.
- Staged source version `0.6.68` for GH-934: MCP `search` can now compile an
  explicit task-aware `RetrievalPlan` from intent, role, risk, budget, and
  superseded-scope inputs; routed searches apply the plan's channel weights,
  graph-expansion choice, rerank bounds, and high-risk raw-fallback abstention
  while returning plan audit metadata.
- Staged source version `0.6.67` for GH-932/GH-931/GH-934: coding-bench remem
  runs now consume the exact persisted production SessionStart ContextAudit,
  embed its payload-free canonical JSON plus plan/policy/hash/degraded/count/
  budget contract, and independently recompute SHA-256 during verification.
  Missing audit evidence is an explicit runtime contract failure, while
  `no_memory` and curated-file controls mark the contract not applicable. The
  retrieval-dependent diagnostic condition now uses the stable CLI/report id
  `remem_seeded_sessionstart`; historical full-body `remem_preloaded` results
  remain explicitly separate and incomparable.
- Staged source version `0.6.66` for memory governance G1: schema v082 adds an
  append-only project-identity alias event ledger plus a current alias registry,
  allowing historical project paths to resolve to one canonical project without
  rewriting capture evidence. A read-only inventory classifies path values with
  filesystem, normalized Git remote, and commit-membership proof; it emits
  digest-bound alias proposals while blocking unproven paths.
- Staged source version `0.6.65` for GH-932: Bundle-backed SessionStart
  emissions now atomically persist a payload-free canonical `ContextAudit`
  beside the existing item-level injection rows. The append-only v081 record
  carries schema/policy versions, plan and audit SHA-256 hashes, degraded and
  truncation state, counts, and token budget/estimate under one
  `injection_run_id`; verified reads detect hash or summary tampering,
  identical retries are idempotent, retention cleanup is bounded, and write
  failures are error-level diagnostics without hiding hook output.
- Staged source version `0.6.64` for GH-932: the experimental MCP
  `context_bundle` tool now accepts a closed, versioned v1 request and returns
  the DB-backed SessionStart `ContextBundle` with complete selection/drop audit
  through both legacy JSON text and MCP `structuredContent`. It validates
  schema, scope, role, risk, and budget inputs, fails canonical partial loads
  closed as audited `blocked` bundles, publishes a typed closed output schema,
  and performs no foreground LLM or network call.
- Staged source version `0.6.63` for GH-932: default `remem doctor` now reports
  a payload-free `Context compiler` capability check for the production
  SessionStart consumer. It exposes the active render/degraded mode plus
  bundle, plan, router, and relevance contract versions; explicit legacy
  rollback warns, invalid configuration fails, and request-specific plan
  inspection remains available through `remem context-plan`.
- Staged source version `0.6.62` for GH-932: healthy production SessionStart
  renders now consume Context Bundle v1 from the same canonical load snapshot.
  The unified plan hashes the effective runtime limits; the bundle owns scope,
  trust, attribution, and relevance, while the established renderer preserves
  exact section/total character boundaries and seals the audit to the final
  stable identities. A regression fixture proves legacy and bundle-backed
  output are byte-identical, and
  `REMEM_CONTEXT_BUNDLE_RENDER_MODE=legacy` is the explicit rollback.
- Staged source version `0.6.61` for GH-933: `remem doctor truth` is the first
  production consumer of the CurrentTruth read model. It reports scoped
  lifecycle mappings, conflicts, abstentions, supersedes links, and invalid
  claim references through a current-schema read-only connection without
  printing claim text.
- Staged source version `0.6.60` for GH-942: summary-derived candidates now
  bind each claim to the strongest captured event that actually supports it
  before deriving trust or persisting evidence. An unrelated low-trust
  `session_stop` no longer poisons a trusted claim's entire rollup window,
  while externally supported claims and transcript-only support remain
  review-gated. Multi-claim candidates persist a deterministic union of their
  supporting event ids.
- Staged source version `0.6.59` for GH-932: the Context Bundle executor now
  has a production path. `compile_session_start_bundle` goes request -> plan
  -> candidates -> bundle against a database connection, where previously
  candidates only ever arrived from tests. Preferences are pulled in through
  their own SessionStart selection (they never reach `LoadedContext`), so the
  bundle's preferences section is populated rather than silently empty, and a
  canonical load failure fails closed into a `Blocked` bundle carrying
  `truncation_reason=canonical_load_failed` instead of a shorter candidate
  list.
- Staged source version `0.6.58` for GH-958: extraction now captures failure
  trajectories as preventive guardrail lessons instead of only successes. The
  candidate prompt asks for compile/test error chains, reverted approaches, and
  repeated same-topic corrections, and a lesson block may declare
  `<outcome>success|failure</outcome>`. The outcome persists on the candidate
  row (new `outcome` column, v080) so it survives the pending_review round-trip
  and lands in `memory_lessons.outcome_kind` with the matching success/failure
  count. On injection, failure lessons render with an explicit
  `guardrail — this failed before:` marker and co-present with success lessons
  in one section, so the model reads dead ends as warnings rather than guidance.
- Staged source version `0.6.57` for GH-956: the extraction LLM pass now
  produces subject-predicate-object temporal facts over the closed
  FactPredicate vocabulary, persisted through the candidate row (new `facts`
  column, v079) and written into `memory_facts` inside the promotion
  transaction. Contradicting facts supersede (valid_to closes) instead of
  being deleted, identical triples are idempotent, and validity timestamps
  ground on evidence events rather than model output, so the bi-temporal
  read side finally receives production data.
- Staged source version `0.6.56` for GH-952: hook and one-shot CLI
  invocations now run on a current_thread tokio runtime (only mcp, api,
  worker, eval-e2e, bench, and dream keep the multi-thread pool), hook
  processes cap the embedding network deadline at 2s by default
  (`REMEM_EMBEDDINGS_HOOK_TIMEOUT_SECS`) so a slow provider degrades through
  the configured fallback chain instead of hanging SessionStart behind the
  30s API timeout, and the rerank model cache moved from thread_local to
  process-level so long-lived MCP/API servers load the model once per
  process instead of once per worker thread.
- Staged source version `0.6.55` for GH-957: the semantic vector channel now
  serves globally nearest candidates from a statically linked sqlite-vec
  `vec0` index (one mirror table per embedding dimension profile) instead of
  sampling at most 4096 embedding ids, removing the recency/bucket truncation
  that made older relevant memories unreachable. Backfill advances one
  512-row batch per connection open, writers dual-write, the inactive-profile
  prune drops retired mirror tables, and any unavailable extension, table, or
  incomplete backfill degrades to the previous bounded brute-force cosine
  scan without changing the search contract.
- Staged source version `0.6.54` for GH-947: the usage feedback channel is
  gray-released at the calibrated default `USAGE_WEIGHT = 0.25`, closing the
  M5 write-only loop on both the SessionStart injection path and the search
  path through a single `SearchWeights::production()` constructor. The
  `eval-weight-grid` shadow evidence (eval/weight-grid/report.json) shows zero
  scored-query, abstention, and top-result movement for every shadowed usage
  weight against the fixed zero-usage baseline, and eval-gates golden
  recall@5 moves 0.5167 -> 0.5246 (no drop). Operators can roll back
  byte-identically with `REMEM_USAGE_WEIGHT=0`; invalid override values log an
  error and keep the calibrated default. The usage shadow report now always
  measures against an explicit zero-usage baseline so the artifact keeps
  documenting the channel's effect after the default flip.
- Staged source version `0.6.52` for GH-991: the public
  `adversarial-policy` `remem_default` condition now records captured events
  and runs deterministic fixture responses through the production observation
  extraction, memory-candidate governance, auto-promotion, and retrieval path.
  Active claims, reviewable candidates, and summary inputs are read from the
  resulting SQLite state instead of inferred from fixture flags. Run artifacts
  name their verification path and measurement source, use the production
  source-event scanner configuration, and report source matches separately
  from generated-surface quarantine (including the opaque-payload distinction).
  Comparative direct-memory fixtures remain explicitly labeled baselines.
  Poisoning-quarantined observations are now excluded from candidate batches,
  closing the production drift that the new end-to-end evaluator exposed.
- Staged source version `0.6.51` for GH-990: new explicit
  `remem dream-backfill` command closes the stock half of the Dream poisoning
  boundary. Pre-v076 Dream-merged active memories (identified by
  `session_id='dream'` plus the v060 default trust class) are re-scanned with
  the same generated-surface scanner and calling convention as the forward
  path. Scanner hits are archived, bound to a `dream_quarantine_artifacts`
  row via the new `backfill_memory_id` column (v077), and enter the existing
  review queue; approving such a candidate restores the same memory in place
  instead of promoting a copy, while rejection leaves it retired with a full
  audit trail. Non-hits only have `source_trust_class` backfilled to
  `external_content` without touching recency signals. Planning is the
  default — `--apply` is required for any write. JSON and human reports expose
  a plan digest; `--expect-plan-digest` can bind apply to a reviewed rehearsal,
  and the complete plan is rechecked inside one immediate transaction before
  the immutable quarantine ledger is written.

### Changed
- GH-932/GH-934: one plan type instead of two. `ContextPlan` and the
  `context_bundle` planner are removed; `RetrievalPlan` now carries both the
  retrieval-source side (`channel_plans`) and the output-section side
  (`output_sections`, `section_budgets`) under a single `plan_hash`, so the
  router's intent, rerank, trust, and abstention policy reach the executor
  instead of stopping at a plan nothing could execute. `SessionStart` becomes
  a routable explicit intent with its own channel set; keyword resolution
  still cannot produce it, since a session start is a host lifecycle event
  rather than something inferable from task text.

### Fixed
- Staged source version `0.6.71` for GH-942: Codex Stop capture now
  materializes timestamped conversation turns as first-class captured
  `message` events before the `session_stop` row. Genuine user turns retain
  `user_prompt` trust; assistant turns remain `external_content` because the
  flattened transcript cannot prove local provenance, and meta/XML control
  turns receive no trusted identity. The batch resolves Git branch state once,
  and transcript-derived prompt events share the existing 128-message/64 KiB
  aggregate budget.
- Staged source version `0.6.53` for GH-992: hook-originated compatibility
  `events` rows now carry their canonical `captured_events.id`. Capture,
  extraction-task enqueue, Git evidence, and the compatibility projection
  commit or roll back together; exact retries reuse the one linked row,
  projection payload drift fails closed, and spill replay cannot append a
  duplicate after partial persistence. Cursor dual delivery updates that same
  projection under the existing failure-wins rule. Audit-only event writers
  and upgrade-era unlinked history remain valid and untouched.
- The Codex plugin runtime version probe now allows 15 seconds for cold binary
  startup instead of hard-coding 3 seconds. `REMEM_RUNTIME_VERSION_TIMEOUT_MS`
  can set a different positive-integer bound, and invalid explicit values fail
  with a visible diagnostic instead of silently selecting a fallback.
- Staged source version `0.6.50` for GH-947: the SessionStart injection path
  now applies `SearchWeights::usage`. It previously fused only its fts, entity,
  temporal, fact, and vector channels, so `usage` was validated but never
  scored — raising `USAGE_WEIGHT` would have reordered `retrieval::search`
  results while leaving injected context byte-identical. Usage reranks the
  candidates the retrieval channels already surfaced and never introduces a
  memory on its own. The default weight stays `0.0`, so this change is
  behavior-neutral until the rollout in GH-947 raises it.

### Added
- Staged source version `0.6.48` for GH-955: memory candidates now use a
  closed low/medium/high risk rubric and claim-level source support instead of
  rejecting an entire candidate for common words such as `not`, `fail`, or
  `token`. Directly supported observation-path failure-and-recovery lessons can
  promote; non-failure lessons, preferences, procedures, credential-token
  variants, auth/authz or destructive semantics, modal or outer-negated claims,
  unsupported claims, and generic imperative instruction content remain
  fail-closed with explicit reasons. The deterministic extraction corpus
  expands from 2 to 27 labeled cases and reports the complete risk-class
  distribution.
- Staged source version `0.6.47` for GH-981: all 14 MCP tools now publish
  explicit titles and truthful read-only, destructive, idempotent, and
  open-world annotations. The 13 JSON tools also publish object-rooted output
  schemas and schema-conforming `structuredContent` while preserving their
  existing text content byte-for-byte; the Markdown timeline report remains
  explicitly unstructured. Router construction fails closed on contract drift,
  and descriptions now disclose access-accounting and summary-quarantine
  side effects.
- Staged source version `0.6.44` for the GH-946 query-scaffolding follow-up:
  temporal parsing now returns exact consumed spans and removes only validated
  time phrases from claim tokens. Conversational request scaffolding and
  localized time expressions no longer inflate claim confidence, while
  malformed, grouped, signed, zero, and overflowing counts fail closed.
- Staged source version `0.6.42` for GH-950: repeated static SQL on the
  SessionStart summary paginator and observation hash/vector dedup funnel now
  reuses rusqlite's per-connection prepared-statement cache. The four static
  staleness statements were already cached; dynamic placeholder SQL and
  one-shot queries remain uncached so they do not churn the bounded cache.
- Staged source version `0.6.41` for the post-merge doctor corrective:
  plaintext-residue inspection now accepts only strictly validated internal
  Hugging Face snapshot pointers whose same-repository blob is a regular file.
  The pointer is never followed and the blob remains independently scanned;
  malformed, broken, absolute, escaping, or blob-symlink aliases keep the
  inspection explicitly incomplete instead of producing a false healthy
  result. Windows reparse points are rejected before recursive descent,
  truncated files inside the managed backups tree remain fail-closed, and a
  regular file occupying the `backups` path is still content-inspected.
- Staged source version `0.6.40` for the post-merge event-retention
  corrective: automatic cleanup deletes only the seven explicitly ephemeral
  event kinds. Governance, scope-cleanup, and all future unknown event kinds
  remain durable audit history by default instead of being silently classified
  as disposable.
- Staged source version `0.6.39` for GH-953 stage S1: SessionStart injection now
  scores from `SearchWeights` instead of eight private scoring constants in
  `hybrid_context.rs` that duplicated it and had already drifted — the injection
  path had no `graph` channel and no `usage` channel. Because
  `eval-weight-grid` tunes `SearchWeights` and injection never read it, the
  evaluation harness was optimizing a path users do not take. Channel SQL and
  post-fusion behavior are unchanged; `docs/specs/GH953/TECH.md` stages the
  remaining convergence work so each ranking-visible change lands with its own
  evaluation delta.
- Staged source version `0.6.38` for GH-951: the owner-trace exclusion query is
  a `NOT ... OR ...` scan no index can serve, and it ran on every SessionStart
  even though its only consumers are debug output and `governance_eval_snapshot`
  (which passes `collect_diagnostics = true`). It is now gated, removing a full
  table scan from the non-debug hot path. The indexed `id IN (...)` lookup stays
  unconditional because `owner_counts` feeds `ContextRenderStats` and
  `remem context --status`; only the per-row trace construction is gated. A test
  asserts that a non-debug load keeps identical owner counts and memory
  selection while reporting no traces.
- Staged source version `0.6.37` for GH-949: every database connection now
  applies tuned SQLite pragmas from one place. `cache_size=-65536` (64 MiB),
  `synchronous=FULL`, and `temp_store=MEMORY` join the existing WAL,
  foreign-key, and busy-timeout settings; SQLCipher disables mmap, so page
  residency matters far more here than on a plaintext store, and every cache
  miss otherwise costs a `pread` plus an AES decrypt. `FULL` preserves the
  prior power-loss durability by default; `REMEM_SQLITE_SYNCHRONOUS=normal`
  makes the WAL latency/durability tradeoff explicit. Read-only connections
  take a narrower set that omits write-only pragmas. Both tuning overrides
  fail before opening the database on invalid or non-Unicode values.
  `REMEM_SQLITE_CACHE_KIB` accepts 1 through 1048576 KiB. The three
  `open_configured_*` helpers no longer carry separate pragma strings.
- Staged source version `0.6.36` for the GH-946 post-merge corrective:
  automatic E5 downloads now use the exact immutable Hugging Face revision
  evaluated by the checked-in provider evidence, while presets without an
  approved revision fail before any network request. CLI help, README, and the
  runtime error now distinguish automatically downloadable E5 from BGE support
  through an already installed verified cache. Exact-entity retrieval also
  grounds the irregular `build` / `built` / `builds` / `building` family
  without matching unrelated predicates, and the graph-decision fingerprint is
  regenerated against the corrected implementation.
- Staged source version `0.6.35` for the GH-934 post-merge corrective:
  Retrieval Router plans now preserve the caller's `include_superseded`
  temporal scope across explicit, keyword-fallback, and default-fallback
  intents. Filters, freshness policy, and history-channel validity share the
  same caller-controlled value, so intent classification cannot silently
  expand access to superseded evidence.
- Staged source version `0.6.34` for GH-946: `Auto` now prefers an explicitly
  downloaded, verified `multilingual-e5-small` local embedding model over
  feature-hash when no remem-specific API key is configured. Provider/model
  switches gain an actionable doctor backfill hint. The default-k provider
  comparison now records the verified model artifact digest, preserves
  abstention and existing-slice budgets, and shows the local paraphrase
  improvement while separating cold-start latency from warm-query p95. A
  two-stage evidence gate prevents unsupported vector-only tails without
  suppressing semantic fallback behind weak lexical hits. Local manifests now
  bind active HF snapshot symlinks and resolved blobs, released schema-v1
  installs migrate offline, runtime sessions are process-wide singleflight,
  and artifact-qualified model ids prevent old/new weight revisions from
  silently sharing vector coverage. Windows model staging, private runtime
  caches, and lock files now use protected owner-only ACLs, reject reparse
  points, and verify 128-bit file identities under native Windows CI. Windows
  custom/shared model roots fail closed instead of accepting a weaker trust
  boundary.
- Staged source version `0.6.32` for GH-945: workers now schedule one
  database-global lifecycle cleanup after a durable 24-hour cooldown and claim
  it through a dedicated lane. Each run atomically expires memory TTLs, advances
  inactive workstreams, removes only explicitly ephemeral old events, archives
  stale memories, and deletes old compressed sources only when their supported
  canonical v2 content/provenance snapshot remains sufficient after
  compression-time revalidation. Exact historical v1 links are upgraded to v2
  in the deletion transaction before old sources are removed; malformed or
  changed v1 data remains fail-closed. Mutable lifecycle/access metadata is
  checked separately or excluded by contract. Automatic cleanup cannot purge
  archived failures. A maintenance ledger and doctor check expose success,
  redacted failure, retry, overdue, and stalled-lease state; bounded SQL batches
  keep large ID sets below SQLite parameter limits.
- Staged source version `0.6.31` for GH-948: indexed source-anchor staleness
  lookup. Schema v074 adds a commit-epoch expression index and a
  trigger-maintained commit-file relation; link-first and split epoch/ID
  queries preserve branch, path-overlap, and equal-timestamp semantics without
  scanning pre-anchor project history. SessionStart now reports a dedicated
  `load_staleness_labels` phase and includes a 50,376-commit ignored benchmark.
- Staged source version `0.6.30` for GH-943: ordinary workers now drain
  eligible residual `pending_observations` into the current capture/extraction
  pipeline only when no current extraction task is ready. The bridge admits at
  most 25 oldest known-host rows once per `worker --once` process or once per
  60-second daemon interval, handles pending/expired-processing/due transient
  and historical archived transient rows, uses per-row immediate transactions
  plus replay savepoints, prepares Git metadata before taking the SQLite write
  lock, revalidates the source snapshot after locking, and records capped
  exponential backoff without rewriting first-failure/archive history on
  replay errors. A zero-progress yield to newly arrived current work keeps the
  once/interval admission available, while partial progress consumes it.
  Doctor keeps deferred archived transient rows visible with their earliest
  retry epoch, reports due automatic backlog with executable `remem worker
  --once` guidance, and marks archived permanent/unknown-host rows
  `admin-required`, listing a bounded oldest-first candidate set with concrete
  exact-ID recovery commands instead of relying on the global recent-failure
  list. The new exact `remem pending recover-archived --id` command supports
  dry-run, requires an explicit host for stored unknown identity, and clears
  failure/archive state only after transactional replay succeeds.
  Deterministic two-connection coverage proves Git enrichment leaves the
  database write lock available, and process-level kill/restart coverage proves
  a durable failed backlog still drains to zero after restart. This staged
  version follows `0.6.28` (#962) and `0.6.29` (#960); merge those PRs first.
- Staged source version `0.6.27` for GH-934: Retrieval Router v1
  deterministic plan compilation. New `src/retrieval_router/` module with
  a versioned `RetrievalPlan` (per-channel enabled/limit/weight/max
  contribution/degradation, scope filters, rerank/trust/freshness/
  abstention policy placeholders, token budget, policy version, stable
  SHA-256 plan hash), six routable `ContextIntent` variants (resume_work,
  explain_decision, debug_failure, apply_preference, review_change,
  explore_history) with test-locked per-intent channel mappings, and
  deterministic intent resolution (explicit caller intent wins, keyword
  rules are the only fallback, unclassified tasks conservatively fall
  back to explore_history). New `remem context-plan --task ... --json`
  debug command prints the resolved intent, enabled/disabled channels,
  filters, budgets, policy version, and reason codes — never memory
  contents; no LLM or network call on any router path. Wiring the plan
  into retrieval execution, rerank mechanics (GH-851), and per-intent
  golden-fixture ablation remain follow-up work on GH-934.
- Released in `0.6.26` for GH-933 Phase A: read-only CurrentTruth
  projection module (`remem::truth`). Versioned Evidence/Claim/Relation/
  CurrentTruth read DTOs, a baseline lifecycle mapping (publication /
  validity / retention plus policy visibility) for memories, observations,
  user-context claims, and memory candidates, and a deterministic resolution
  policy: project/branch claim filtering, `as_of` filtering, explicit
  supersedes over recency, verified evidence over model-generated,
  `Contradicted` for unresolved conflicts, and abstention instead of guessing.
  No writer, schema, or context-path changes; pending v2 hardening and later
  Context Bundle work remain tracked by GH-933. Contract:
  `docs/specs/GH933/`.
- Staged source version `0.6.25` for GH-932: Context Bundle v1 internal
  contract. New `src/context_bundle/` module with versioned
  `ContextRequest` / `ContextPlan` / `ContextBundle` / `ContextAudit`
  serde-JSON DTOs, a deterministic `plan(request)` / `execute(plan, inputs)`
  split that reuses the SessionStart relevance policy
  (`sessionstart_significant_token_v1`) and section limits, a stable
  SHA-256 plan hash with no time or randomness inputs, machine-readable
  selected/dropped audit reasons with token estimates, and
  `full` / `canonical_only` / `blocked` degraded modes. Experimental
  internal Rust API only; MCP/REST endpoints, DB-backed executor wiring,
  doctor plan summaries, and benchmark artifact hashes are follow-up work
  on GH-932. Existing SessionStart rendered output is unchanged.
- Staged source version `0.6.24` for GH-855: capture/extraction-path poisoning
  defense. Session rollups now compute a deterministic combined verdict over
  the captured source events and every model-generated summary field before
  persistence; a hit stores a durable `quarantined` summary row (schema v072:
  `poisoning_status`, quarantine stage/field/event/pattern metadata, block
  counters) and withholds all model-visible side effects (topic segments,
  candidates, native memory) — source hits cannot be laundered by a clean
  summary. All model-visible summary sinks (SessionStart recent sessions,
  MEMORY.md native sync, observation/summarize prompt context, user-context
  extraction/recall/activity, git/MCP commit trace, summary queries) exclude
  quarantined rows in SQL and re-scan the fields they expose right before
  use, quarantining legacy rows on first hit (fail closed on errors).
  Observation persistence and legacy `finalize_summarize` run the same
  scanner; poisoned observations land as `poisoning_quarantined` and never
  enter active queries. `remem doctor`, `remem status`, and HTTP `/status`
  gain a `poisoning_defense` section (pattern set version, candidate/summary/
  observation quarantine counts, legacy-unscanned summaries, block counts,
  injection drops). The public `adversarial-policy` suite is revised to v2
  with `instruction_injection` (EN/ZH), `authority_claim`, `opaque_payload`,
  and `benign_quoted_instruction` categories scored through the production
  pattern scanner instead of hardcoded zeros; quarantine wins over
  `retention_allowed`.
- Staged source version `0.6.23` for GH-850: write-side contextual enrichment
  (`retrieval_text` equivalent) on the single index-only `search_context`
  surface. Migration v072 adds enrichment identity/claim/lease/failure columns
  plus the `retrieval_enrichment_compatibility` singleton with a monotonic
  security-policy floor; the rebuilt `memories_au` trigger persists an empty
  deterministic fallback (and drops stale vectors) whenever a raw canonical
  update bypasses the production writers, and every FTS rebuild reads the
  final persisted row. The idle worker lane generates one bounded
  context-sentence + synonym-keyword block per memory through the existing
  memory AI profile (strict closed-JSON parser, secret redaction, poison
  re-scan, durable claim/lease/attempt CAS, exponential backoff capped at 15
  minutes); FTS and the vector channel consume the same snapshot via the
  versioned `memory-index-v2` passage hash. `remem doctor` gains a
  `Retrieval enrichment coverage` check (floor/epoch/state, ready/pending/
  failed counts, source-identity drift, vector consistency), and eval gate
  thresholds support strictly-positive `min_value` floors for the paraphrase
  slice. Canonical `title`/`content` bytes and injection/API/export payloads
  are unchanged; hooks and foreground writes never wait on generation.
- Staged source version `0.6.22` for GH-852: host-native memory data sources.
  New `remem import codex-memories` performs a one-way, read-only import of
  Codex CLI rollout-summary memories (`codex-rollout-summary/v1`, fingerprint
  frozen against codex-cli 0.145.0) into the candidate review queue: two-phase
  dry-run/apply bound by `--expect-plan-digest`, pre-persistence secret
  boundary that blocks the whole batch, instruction-pattern quarantine,
  content+route idempotent identity (rename-safe re-runs), verified-cwd
  project routing with a Codex tool-owned `search_only` fallback, and a single
  all-or-nothing transaction that only ever lands `pending_review` /
  `quarantined` candidates — never active memories. `remem doctor` gains a
  Codex native memories check (not_configured / ready / unreadable /
  unsupported_format, no body output) and reports the Claude native-bridge
  state. Claude native topic-file ingestion is closure-audited: remem's own
  `remem_sessions.md` is no longer self-ingested, topic files route into
  `memory_candidates` as `external_content` (pending review, never
  direct-active), and ingestion failures propagate to the hook exit status
  instead of warning-only. The Claude `autoMemoryDirectory` delivery bridge
  stays `hook_only`/no-go pending SP852-T1 real-host PoC evidence; no user
  `~/.claude` or `~/.codex` surface is written by default
  (docs/research/gh852-host-native-memory-poc.md).
- Staged source version `0.6.21` for GH-851: opt-in second-stage local
  cross-encoder rerank. After RRF fusion, eligibility filtering, and
  source-anchor demotion, standard `remem search` (including API/MCP service
  callers) and the SessionStart implicit query share one rerank stage that
  re-scores the fixed top-N baseline candidates with a locally installed
  fastembed/ONNX cross-encoder and returns a fixed top-k order
  (score desc, baseline rank asc, id asc; `verify-before-trust` candidates
  hard-partitioned last after a successful rerank only). Rerank is
  default-off (`[rerank] enabled = true` or `REMEM_RERANK_ENABLED=1`);
  models install only via the explicit `remem reranker download` command
  into a dedicated reranker inventory with a byte/SHA-256-verified manifest
  — search, hooks, API/MCP, and doctor never download. Missing, corrupt,
  load-failed, inference-failed, deadline-exceeded, or cancelled states
  fall back atomically to the complete RRF baseline with a stable closed
  `disabled_reason` (error-level logged), surfaced in search explain
  (`rerank` stage with `rerank_model_load`/`rerank_inference`/`rerank_total`
  phase timings), SessionStart render stats, `remem reranker status`, and a
  new doctor check that fails for enabled-but-broken models and passes for
  explicit off. A rerank A/B promote gate (`src/eval/rerank.rs`) enforces
  paraphrase/associative and combined MRR@10 / Hit@5 non-regression plus a
  preregistered `>= 0.05` primary-metric improvement before any default-on
  decision; the paired artifact must carry commit, dataset hash, and model
  manifest hash. Default-on remains gated on maintainer-approved runtime
  evidence (local A/B artifact and cold/warm SessionStart p95 budgets).
- Staged source version `0.6.20` for GH-824: Cursor install host surface.
  `remem install/uninstall --target cursor` (plus `auto`/`all` selection on
  macOS/Linux) manages exactly the user-level `~/.cursor/hooks.json` and
  `~/.cursor/mcp.json` through a strict whole-document Cursor v1
  hooks/MCP schema validator, an exact managed `mcpServers.remem` stdio
  entry, a versioned non-sensitive install receipt under
  `[memory_ai.hosts.cursor]`, and a secure staged writer (owner-only temp
  before the first byte) with per-target final comparison, read-back, and
  compensating rollback — not a cross-file atomic transaction; foreign JSON
  is preserved semantically for the validated plan snapshot and observable
  concurrent edits abort, while the compare-to-rename window remains a
  documented residual risk. Because the merged GH-823 runtime has no total
  delivered-failure policy, froze generic MCP ownership, and Cursor
  summarize stays blocked on GH-825, contract v1 installs no Cursor hook
  entries yet: install output says so explicitly and never claims automatic
  Cursor memory is enabled. `remem doctor` gains a Cursor check reporting
  detected/configured/configured_mode/malformed/partial_state/drift/
  collision, per-capability effective status from the PR #914 evidence,
  `hook_failure_policy: host_continues`, and the fixed line
  `session-init: not supported on cursor`. Windows/UNC stays fail-closed
  (explicit cursor/all) or non-fatally skipped (auto). Claude Code and
  Codex install/uninstall/dry-run/doctor behavior is unchanged.
- Staged source version `0.6.19` for GH-825 (with GH-823 SP823-T5): lossless
  Cursor transcript capture. `remem summarize --host cursor` now performs the
  full Cursor Stop validation (approved status set `completed|aborted`,
  non-empty string `generation_id`, `loop_count` normalized from an exact
  non-negative integer, canonical Stop key
  `(session_id, generation_id, loop_count)`), takes a Stop-time snapshot of
  the Cursor JSONL transcript, and fully validates it against the observed
  PR #874/PR #914 grammar: `{role,message}` records with `text` and assistant
  `tool_use` content blocks plus standalone `turn_ended` boundary records
  (including the aborted-turn error form). Every record gets a zero-based
  physical ordinal before any filtering; `turn_ended` records never enter the
  raw/prompt projections. The validated, redacted IR rides the durable
  `session_stop` capture payload (`cursor_capture` marker) and drives bounded
  SessionRollup prompt evidence; `transcript_path` never leaves the hook, so
  the Claude/Codex transcript reader and path-based drain are unreachable
  from Cursor and the worker never reopens the path. Missing/null/blank/
  untrusted/unreadable/oversized/changed/corrupt/empty transcripts degrade
  explicitly to payload-only with machine-readable
  `degraded/<reason>` markers and `capture_drop_events`
  (`cursor_transcript_<reason>`) diagnostics — the Stop and previously
  captured tool evidence are never lost. No schema change; Claude Code and
  Codex behavior is unchanged.
- Staged source version `0.6.18` for GH-823 (SP823-T3/T4): first-class Cursor
  hook I/O protocol. `cursor` joins the exact closed hook-host set
  (`claude-code`, `codex-cli`, `cursor`); every Cursor entrypoint reads stdin
  through a bounded 1,048,576-byte reader and validates payloads fail-closed
  against the Cursor 3.12.17 evidence (PR #914). `remem observe --host cursor`
  captures generic `postToolUse`/`postToolUseFailure` events with
  `tool_use_id` as the canonical per-call key, stores the observed failed-Read
  path under the existing `captured_events.event_type = "cursor_tool_failure"`
  discriminator (no schema change), and keeps MCP-specific events unregistered
  (B-016 generic ownership). `session-init --host cursor` is explicitly
  unsupported, Cursor summarize stays fail-closed until GH-825's transcript
  reader, and Cursor 3.12.17 session-start injection remains disabled with no
  post-tool context command added. Cursor `user_email` and other
  non-canonical PII are dropped at the payload boundary. Claude Code and
  Codex protocol behavior is unchanged; the only tightening is that explicit
  hook `--host` aliases and arbitrary values now fail closed.

### Changed
- Staged source version `0.6.45` for GH-982: the existing 14-tool MCP surface
  now describes read/write behavior, response shapes, defaults, failure modes,
  and selection boundaries for current state, search, contextual recall,
  timelines, detail reads, reports, and workstreams. Public tool names and
  runtime behavior is unchanged except that `update_workstream` now rejects
  unknown status strings and calls without any update field instead of silently
  normalizing or touching the row; `workstreams` rejects unknown status filters,
  blank timeline/multi-hop queries fail explicitly, and timeline-report metric
  query failures propagate instead of being rendered as zero.

### Fixed
- Staged source version `0.6.49` for GH-969: Dream now scans every generated
  decision surface, including the combined title/content render and no-merge or
  conflict reasons, before persistence. Poisoned output atomically becomes a
  route-scoped quarantined candidate plus a versioned cluster-bound artifact;
  token-aware review binds current source versions and canonical payloads and
  can supersede only the exact acknowledged source set. Every later semantic
  decision—including a clean no-merge or conflict—atomically invalidates older
  review candidates for the same cluster.
  Every decision now rechecks source payload, TTL, state-key currentness, and
  suppression in its immediate write transaction. Clean generated memories are
  external trust, and cluster-external state-key/semantic reuse rolls back.
  External-candidate dedup now uses an immutable, atomic source/destination
  identity ledger, so identical native content is isolated across projects and
  retries remain idempotent after review or edits.
- Staged source version `0.6.43` for GH-954: entity, temporal, fact, LIKE
  fallback, and graph channels now contribute pure weighted RRF instead of
  feeding reciprocal rank back as a synthetic normalized score. FTS, vector,
  and opt-in usage retain calibrated strength signals; equal-score FTS falls
  back to rank-only fusion. Search explain output reports the computed
  pre-/post-fusion score identity while preserving the existing public Rust
  explain-struct field layout. Both search and SessionStart injection now
  reject non-finite vector thresholds or distances before channel filtering.
- Staged source version `0.6.33` for GH-944: `remem doctor` now detects
  plaintext SQLite residue across the active data and backup locations,
  reports inspection failures instead of silently skipping them, and bases
  cleanup guidance on the live database state. Release metadata remains
  `unreleased`.
- Staged source version `0.6.17`: AI HTTP calls now reuse one process-wide
  `reqwest::Client` (connection pool + TLS config) via a `OnceLock` instead of
  rebuilding a client on every call. The timeout is a compile-time constant, so
  a single client serves every call.
- Staged source version `0.6.16`: `run_migrations` now skips the
  `BEGIN IMMEDIATE` write-lock transaction when the database schema is already
  current, so read-heavy callers that open a fresh connection per request (the
  REST API and MCP server) no longer take the database write lock on every
  connection open. The check is read-only and WAL-concurrent; the full
  migration path is unchanged when any migration is pending or a schema
  invariant is violated.
- Staged source version `0.6.15` for GH-813: the preference-rule compiler now
  applies one typed, centralized eligibility policy and admits a global-scope
  preference only for the canonical
  `owner_scope='user'` / `owner_key='user:default'` / no-project-target owner
  tuple, and unknown owner/scope/risk/review/trust values fail closed instead
  of compiling. The sweep-project projection uses the same closed predicate.
  Adds an exhaustive behavior-based eligibility matrix (positive global
  baseline, one independent negative per dimension, unknown-value fail-closed
  cases, independently mutable candidate and reinforcement risk, the
  wrong-owner-scope / wrong-owner-key / project-target regressions, and a
  cross-state case). No hook or evaluator change. Release metadata stays
  `unreleased` until publication. Also keeps the unchanged ingest eligibility
  predicate warning-free under Rust 1.95 without changing its behavior.
- Staged source version `0.6.13` for GH-900: the checked-in graph-decision
  report now carries a deterministic length-prefixed SHA-256 fingerprint of
  `eval/golden.json` and every evaluator/retrieval source that can affect the
  result, with a guard test that rejects a stale report. Completes the GH-853
  focused regression matrix. No traversal ranking or production behavior
  change. Release metadata stays `unreleased` until publication.
- Staged source version `0.6.12` for GH-720 SP720-T5: `remem raw messages`
  exports one exact `(source_root, project, session_id)` tuple with full stored
  content, stable `(created_at_epoch, id)` ordering, and selector-bound
  snapshot cursors for lossless downstream session consumers. Release metadata
  stays `unreleased` until publication.
- Staged source version `0.6.11` for GH-853: standard memory search now expands
  eligible FTS/vector seeds through bounded trusted `graph_edges` paths, with
  deterministic RRF ordering, explicit empty reasons, and a same-head literal
  associative gate that records full gain without non-associative regression.
  PPR and direct context traversal remain deferred. Release metadata stays
  `unreleased` until publication.
- Staged source version `0.6.10` for GH-854: SessionStart now preserves Core,
  Preferences, and Workstreams while applying one deterministic relevance
  budget to Lessons, non-Core MemoryIndex entries, and Sessions. Footer,
  per-item audit, and latest-session status expose the selected threshold and
  closed drop reasons; `REMEM_CONTEXT_RELEVANCE_K=0` restores legacy selection.
  Release metadata stays `unreleased` until publication.
- Staged source version `0.6.9` for GH-860: the structural force-push evaluator
  recognizes supported Git-for-Windows `.exe` shell basenames, binds static
  shell `-c` positional operands, and resolves a function-shadowed `unset`
  before applying builtin state changes. Paired fixtures preserve nearby
  allowed forms. Release metadata stays `unreleased` until publication.
- Staged source version `0.6.8` for GH-871: raw transcript ingestion now uses
  path-stable metadata-first identities and lossless occurrence ordinals;
  validated read-only raw queries avoid migration-lock contention, session
  JSON includes role counts, and bounded aggregate-only reconciliation proves
  fixed-window archive parity without exposing transcript data. Release
  metadata stays `unreleased` until publication.
- Staged source version `0.6.7` for GH-882: memory-candidate extraction now
  normalizes the model-emitted `fact` alias to `discovery` without weakening
  the legal observation vocabulary, and both prompt layers explicitly direct
  factual findings to the canonical type. Release metadata stays `unreleased`
  until publication.
- Staged source version `0.6.6` for GH-880: the authenticated native API now
  advertises safe candidate detail/review, five independently gated read
  resources, and recoverable memory archive/restore. Typed cursor, redaction,
  optimistic-version, idempotency, audit, and current-provenance contracts are
  covered by native smoke and regression gates; permanent Web delete remains
  unavailable. Release metadata stays `unreleased` until publication.
- Staged source version `0.6.5` for GH-880 SP880-T1: schema v70 adds
  fail-closed migration recovery, Web-visible resource versions, an
  idempotency replay ledger, and stable cursor foundations without advertising
  unfinished endpoints or capabilities.
- Staged source version `0.6.4` for GH-864: archived quarantined extraction
  ranges can be validated only by an exact dual-confirmation dry-run and
  recovered only by a singleton-locked worker that atomically requeues and
  claims one task under an explicit AI profile. Non-successful or interrupted
  exact attempts return to archived quarantine instead of entering the normal
  daemon queue.
- Staged source version `0.6.3` for GH-864: operators can explicitly
  acknowledge and retry one quarantined extraction replay range by exact ID;
  dry-run and execution share the same transactional eligibility checks while
  default exact retry and every batch retry continue to exclude quarantine.
- Staged source version `0.6.2` for GH-864: transcript evidence truncation is
  stable across replay, Git branch/commit probes use bounded process-group and
  pipe-reader cleanup, exhausted extraction ranges support exact-ID
  list/retry/quarantine with terminal task evidence, and rollup topic keys
  normalize punctuation without rewriting legacy snake/kebab identities.
- Staged source version `0.6.1` for GH-861: project identity now delegates to
  Git whenever `GIT_COMMON_DIR` is set, so invalid or redirected common-dir
  layouts fail closed instead of being mistaken for plain marker discovery.

### Added
- Staged source version `0.6.0` for GH-684 SP684-T10: `remem doctor`
  announces that `pending_observations` is deprecated and cannot be removed
  before remem 0.7.0. Non-empty stores are directed to preview with
  `remem pending migrate-legacy --dry-run` and then apply with
  `remem pending migrate-legacy`. The removal window does not begin until the
  0.6.0 release is published.
- Staged source version `0.5.214` for GH-671 T7: repeated-correction fixtures
  cover package-manager choices, forbidden commit trailers, and forbidden
  commands; one Brush AST execution model closes wrapper, quoting, function,
  mirror-push, and arithmetic-substitution bypasses while the release hook
  remains within the fixed 1 ms delta and 15 ms enabled-p95 budgets.
- Staged source version `0.5.213` for GH-844: CI and local PR preflight now
  fail on clippy warnings across all Cargo targets, with the existing 11
  test-target lints fixed without suppressions or runtime behavior changes.
- Staged source version `0.5.212` for the GH-720 T1 follow-up: transcript
  ingestion now streams JSONL records with bounded memory, preserves captured
  byte boundaries, and rolls back already-inserted rows when a later read or
  UTF-8 failure makes the file incomplete. GH-720 remains open for its manual
  and cross-repository phases.
- Staged source version `0.5.211` for #720 query parity: the MCP `search_raw`
  tool and CLI `query raw` now share one raw-query assembly path, aligning the
  JSON envelope and date-only `until` bounds across surfaces.
- Staged source version `0.5.210` for GH-684: frozen legacy-surface writes now
  fail doctor instead of remaining warning-only, while the retirement contract
  fixes the 0.6.0 announcement and no-earlier-than-0.7.0 guarded-drop window.
- Staged source version `0.5.209` for #818: job enqueue, claim, lease
  transitions, migration reconciliation, and failure recovery now enforce
  database-atomic active identities and fail closed on conflicts while
  preserving actionable diagnostics and deterministic Dream replay semantics.
- Staged source version `0.5.208` for #819: memory test fixtures now execute
  the canonical v020 FTS migration instead of copying active-only triggers;
  regression coverage keeps stale and archived rows indexed while query
  predicates control visibility, and verifies status transitions, deletion,
  and trigger-schema parity with the production migration chain.
- Staged source version `0.5.207` for #817: observation XML parsing now
  fails closed when the model omits `<type>` or returns an unknown value,
  drops the invalid observation before it can become candidate support
  evidence, and records an error-level `missing_type` or `unknown_type` reason;
  all six declared observation types retain their existing behavior.
- Staged source version `0.5.206` for GH-671 T6: `remem doctor` reports
  compiled-rule artifact presence, rule count, compile and evaluation health,
  and honest per-host enforcement capabilities without exposing rule payloads
  or diagnostic messages.
- Staged source version `0.5.205` for GH-671 T5: Claude Code installs a
  fail-open `PreToolUse` Bash evaluator that emits visible warnings or explicit
  opt-in denials from local compiled artifacts, while the rollout flag disables
  evaluation, PostToolUse remains capture-only, and Codex block enforcement is
  rejected as unsupported.
- Staged source version `0.5.204` for GH-671 T4: `remem rules` lists compiled
  rule provenance and persists disable, enable, and warn/block action overrides
  without writing derived artifacts on the CLI path; independent override
  columns survive concurrent-style updates, artifact regeneration, and worker
  enqueue failures without silent state loss.
- Staged source version `0.5.203` for #796: migration v068 records one
  exact-range SessionRollup follow-up scheduling decision in the same SQLite
  transaction as Compress and Dream enqueueing, so retries cannot replace
  completed, failed, or cooldown-expired jobs, partial enqueue failures roll
  back cleanly, and a genuinely new event range can still schedule new
  maintenance work. Historical exact ranges are marked `legacy_unknown` and
  reported at error level instead of receiving inferred replacement jobs;
  v067 writers that finish after migration inherit the same safe default and
  their pre-upgrade processing leases are requeued;
  newly completed decisions persist the Compress job id plus the exact Dream
  disposition and referenced job id.
- Staged source version `0.5.202` for the MCP registry launch fixes: ships the
  shortened `server.json` description (#808), the real-session recall demo
  assets (#809), and the README hero swap (#810) in a tagged release so the
  `publish-mcp-registry` job can complete its first successful publish.
- Staged source version `0.5.201` for #795: automatic SessionRollup native-memory
  mirroring now reports filesystem failures at error level with project,
  session-row, and exact event-range identity without blocking the persisted
  UserContextCandidate, Compress, or Dream follow-ups; explicit native-memory
  synchronization remains fallible.
- Staged source version `0.5.200` for GH-792 observed commit traceability:
  successful explicit `git commit` results prove SHAs through Claude hook
  output or a byte-bounded Codex transcript, typed evidence is stored atomically
  and survives the shared encrypted spill queue, deterministic extraction
  phases link every commit in the exact claimed range by durable
  `session_row_id`, cross-host raw session collisions remain distinct, retries
  stay idempotent, missing or ambiguous proof never drops the surrounding
  capture, and ordinary Stop events never infer from a later `HEAD`.
- Staged source version `0.5.199` for the GH-671 T3 post-merge corrective:
  archive and reroute operations recompile both affected preference authorities,
  replacement overrides follow normalized predicate identity, global preference
  mutations immediately fan out to registered projects, and failed success
  diagnostics restore or remove the unpublished compiled artifact.
- Staged source version `0.5.198` for #794: SessionRollup now supplies
  one shared byte-bounded, redacted transcript evidence slice to the summarizer
  and candidate support path, deduplicates repeated paths and captured-event
  text, excludes bytes appended after Stop, and persists the exact-range slice
  plus raw-archive completion checkpoint through migration
  `v066_session_rollup_evidence_checkpoint`. Persisted-rollup retries no longer
  depend on a transcript source file after successful raw ingest: per-Stop
  message hashes and parsed citation facts are snapshotted independently of the
  lossy 8 KiB/64 KiB prompt budget for every bounded Stop, including repeated
  path boundaries and Unicode-safe truncation. Early v066 JSON reuses its
  original bounded message/hash on retry to prevent duplicate usage. Legacy Stop
  payloads without a byte boundary use captured conversational events only, or
  fail permanently before AI when no safe fallback exists. Missing, malformed,
  or unusable required bounded snapshots still fail before metadata-only
  summaries can persist.
- Staged source version `0.5.197` for the GH-671 T3 correctness follow-up:
  unique evidence reinforces only the same safe predicate; opposing direct
  saves and cleanup rewrites clear stale provenance while same-predicate
  overrides survive; lifecycle changes enqueue non-lossy compilation and
  periodic sweeps converge canonical projects; reviewed low-risk trusted
  preferences compile with project-over-global precedence; conservative
  classification and config/diagnostic paths fail closed; unchanged artifacts
  do not churn; generated messages remain static; and v065 schema drift guards
  its eligibility columns and index.
- Staged source version `0.5.196` for GH-671 T3 preference rule compiler:
  canonical preference reinforcement state (migration `v065_preference_reinforcement`
  wiring the v062 `memory_preference_reinforcements` table via the apply path) and a
  worker-only rule compiler (`JobType::CompileRules`) with eligibility selection,
  user-override merge, source lifecycle removal, and newest-source conflict resolution.
- Staged source version `0.5.195` for GH-684 Summary upgrade handling:
  migration v064 now rejects non-terminal legacy `JobType::Summary` jobs as
  permanent failures during upgrade, preserving terminal job history and other
  job types while SessionRollup owns session summary output; Stop hooks no
  longer enqueue new Summary jobs, capture-ledger failures spill instead of
  falling back to the retired writer, same-session stale spills are skipped
  after the current stop payload succeeds, raw/citation/failure-lesson Stop
  side effects are owned by the hook path before follow-up enqueue, citation
  recording errors log at error level without blocking follow-up jobs, retryable
  failed Summary rows are frozen during upgrade, doctor/status ignore explicit
  v064 upgrade rejection rows as freeze blockers and actionable failed jobs,
  post-retirement worker rejections stay visible, spill replay compares the
  full host/project/session identity before dropping stale rows, replayed Stop
  captures use stable event IDs so later retry failures stay idempotent,
  replay capture-ledger failures are preserved once by the replay layer instead
  of duplicating active spill rows, old-version daemon heartbeats no longer
  suppress the Stop-hook `worker --once` fallback even when the old daemon
  still holds the legacy singleton lock, migration v064 requeues SessionRollup
  leases claimed before the binary upgrade, workers run extraction tasks before
  Compress/Dream jobs, and worker execution rejects legacy Summary jobs without
  retry if an already-claimed job reaches the runner. SessionRollup side effects
  load the exact persisted event range, and required raw-archive, workstream,
  and native-memory failures keep the extraction task retryable instead of
  completing with missing memory state. Transcript-only Stop payloads now
  snapshot their transcript byte boundary, then record memory citations and
  distill failure lessons after bounded worker-side raw ingest. Coalesced
  rollups drain every covered Stop payload, deduplicate repeated transcript
  paths at the widest captured boundary, and bind summary-candidate evidence
  to the exact persisted event range instead of a later session capture;
  retries of those signals no longer suppress persisted rollup maintenance. A
  versioned once-launch heartbeat prevents overlapping fallback workers while
  an old daemon is still alive during upgrade.
- Staged source version `0.5.194`: `remem status --share` prints a compact,
  screenshot-friendly summary card (totals, today delta, repo URL) that omits
  database paths and project names for safe public sharing.
- Staged source version `0.5.193` for GH-671 preference rule artifact
  foundation: compiled-rule artifacts now have a versioned JSON schema, closed
  v1 predicate enum, deterministic in-memory evaluator, fail-open artifact
  loading, stable project artifact paths, and atomic artifact writes.
- Staged source version `0.5.192` for GH-684 pending queue freeze:
  the dead legacy `pending_observations` enqueue/claim/lease API has been
  removed from the crate while pending admin migration, failure handling,
  doctor, and status tests seed historical rows through an explicit test
  fixture.
- Staged source version `0.5.191` for GH-680 procedure export final guard:
  `remem procedures export` now enforces a runtime CLI invocation guard,
  refuses plugin `skills/` roots before creating missing directories, and
  documents the export command and review-gated overwrite/path semantics in
  the README and current procedure export contract.
- Staged source version `0.5.190` for GH-680 procedure export registry:
  successful review-gated procedure exports now record content/source
  snapshots in `procedure_exports`, and `remem doctor` warns when exported
  drafts drift because the source procedure became inactive, verification
  freshness lapsed, or the active source changed after export.
- Staged source version `0.5.189` for GH-680 procedure export reachability:
  a negative source invariant test now keeps procedure draft export writer and
  renderer entrypoints reachable only from the explicit CLI procedures export
  action, failing if worker, dream, hook, context, summarize, or MCP paths wire
  into the draft writer.
- Staged source version `0.5.188` for GH-680 procedure export writer guard:
  `remem procedures export` now writes reviewable drafts only through the CLI,
  refuses high-context output paths and user-edited targets, and requires
  `--overwrite-generated` before replacing an unchanged generated draft.
- Staged source version `0.5.187` for GH-761 Claude hook integrity repair:
  Claude hook setup now evaluates all five expected hooks, warns during
  SessionStart when registrations are missing or stale, and provides a
  hook-only `remem install --target claude --repair` path that preserves
  third-party hooks and avoids MCP/runtime/token writes.
- Staged source version `0.5.186` for GH-759 final observability and docs:
  `remem status` now reports user-context claim/candidate counts and pending
  block reasons, and the user-facing/runtime specs document the relaxed default,
  strict rollback, unchanged hard gates, governance path, and verification stats.
- Staged source version `0.5.185` for GH-759 relaxed auto-promote safety:
  expanded regression fixtures keep sensitivity, high-risk, third-party,
  assistant-only and mixed non-user source, non-retention, and claim-key conflict
  paths fail-closed under the relaxed default policy.
- Staged source version `0.5.184` for GH-759 auto-promote runtime policy:
  extraction and candidate apply now share the runtime `AutoPromotePolicy`, so
  default user-context auto-promote lowers only the confidence threshold while
  strict mode restores the old 0.9 hard gate and existing safety checks remain
  review-gated.
- Staged source version `0.5.183` for GH-759 auto-promote policy config:
  runtime config now exposes `[user_context.auto_promote]` defaults,
  validation, and a strict rollback policy without changing promotion behavior.
- Staged source version `0.5.182` for GH-760 preference backfill storage:
  dry-run now selects visible user-scope preference memories read-only, and
  `--apply` writes idempotent `preference_backfill` claims with memory source
  refs, governed duplicate skips, stable conversion reporting, documented
  visible-row filters, skip reasons, traceability, and governance rollback.
- Staged source version `0.5.181` for GH-760 user preference backfill CLI:
  `remem user backfill [--json] [--limit <n>]` now exposes a dry-run report
  shape while `--apply` fails closed until the storage conversion slice lands.
- Staged source version `0.5.180` for GH-680 procedure export templates:
  render-time field scanning now blocks secret-like or instruction-pattern
  procedure fields before draft generation, and pinned snapshots cover
  Claude skill, Codex prompt, and runbook draft formats.
- Staged source version `0.5.179` for GH-680 procedure export eligibility:
  the export source loader now reuses fresh procedure verification evidence
  and rejects non-procedure, inactive, expired, suppressed, superseded, or
  insufficiently verified procedure memories before render/write paths land.
- Staged source version `0.5.178` for GH-684 Summary side-effect
  preservation: regression coverage now locks Compress/Dream enqueueing, raw
  archive ingest, memory citations, failure lessons, summary-derived
  candidate finalization, and native-memory sync before Summary writer
  retirement.
- Staged source version `0.5.177` for GH-684 summary writer convergence:
  SessionRollup now persists semantic request, decisions, learned, next steps,
  and preferences fields, and context/user-context readers can consume
  semantic rollup rows while excluding synthetic event-range fallback titles.
- Staged source version `0.5.176` for GH-678 project memory pack completion:
  round-trip export/import identity fixture, pack-origin doctor and `remem why`
  attribution, and README onboarding workflow.
- Staged source version `0.5.175` for GH-678 project memory pack active import:
  safe rows now write active memories with `pack` source trust after
  instruction-pattern scanning, conflicts and quarantines route to review
  candidates, and suppressed/inactive local decisions remain non-resurrected.
- Staged source version `0.5.174` for GH-678 project memory pack import
  dry-run planning: `remem import --pack <dir> --dry-run` validates pack
  manifests/digests and reports add, dedup, skip, conflict, and quarantine
  outcomes without mutating the runtime store.
- Staged source version `0.5.173` for the GH-672 memory poisoning defense
  closure fixture: captured-event instruction payloads now exercise
  candidate quarantine through render absence, and the issue task plan is
  synchronized with the completed security tranche.
- Staged source version `0.5.172` for GH-684 summary writer equivalence:
  field-comparison fixtures now document legacy Summary structured fields,
  SessionRollup range metadata, ownership/context defaults, and cooldown
  side-effect deltas before Summary writer retirement.
- Staged source version `0.5.171` for GH-684 legacy surface visibility:
  status and doctor now report tracked legacy surface row counts, last-write
  epochs, and retire/freeze blockers before later Summary/pending retirement.
- Staged source version `0.5.170` for GH-672 memory poisoning defense:
  source trust metadata, deterministic instruction-pattern quarantine, and
  direct-save trust tagging. The staged line also adds explicit quarantine
  acknowledgement review, render-time poisoned-memory drops, and doctor
  reporting for quarantine/drop state.
- Staged source version `0.5.169` for the GH-671 preference rule
  compilation foundation: disabled-by-default config defaults, canonical
  preference reinforcement state, rule override state, diagnostic state, and
  schema/convergence guardrails without enabling runtime rule behavior.
- Staged source version `0.5.168` for GH-678 project memory pack export:
  deterministic `pack.json`/`memories.jsonl`/`INDEX.md` generation for active
  repo-owned startup memories, fail-loud redaction gating, and focused export
  fixtures.
- Staged source version `0.5.167` for GH-680 procedure export Phase 1:
  `remem procedures list` exposes promoted procedure memories with maturity
  metadata before any review-gated export writer is introduced.
- Staged source version `0.5.166` for GH-684 observation wording: MCP and
  architecture docs now classify `source='observation'` as a current extracted
  observation source instead of a legacy source.
- Staged source version `0.5.164` for GH-673 context stability: total context
  budget enforcement now truncates at stable item boundaries while preserving
  the truncation marker and stats footer.
- Staged source version `0.5.163` for GH-726 local PR preflight: aggregate
  CI gate checks in one command, document it as the PR preflight, and stabilize
  the log lock-open regression test surfaced by the full preflight run.
- Staged source version `0.5.162` for GH-683 review queue throughput:
  review queue health metrics, doctor deadlock findings, batch review
  operations, durable review metadata, and REST blocked-candidate reporting.
- Staged source version `0.5.160` for GH-717 downstream active semantic
  adoption: observation vector dedup, active-model preference embedding
  fallback thresholds, and focused dedup/preference regressions.
- Staged source version `0.5.159` for the GH-716 provider-comparison follow-up:
  optional local/API embedding profile probe failures are recorded as
  unavailable rows instead of aborting the whole report.
- Staged source version `0.5.158` for the GH-716 embedding provider comparison
  eval: EN/CJK paraphrase fixtures, feature-hash/local/API report rows, explicit
  default-flip criteria, and the recorded no-flip decision.
- Staged source version `0.5.157` for the GH-715 local semantic embedding
  runtime slice: fastembed-backed local model download/status, explicit
  active-profile backfill/prune, hook-safe missing-model deferral, and
  verified model manifests.
- Staged source version `0.5.156` for the GH-715 multi-model memory embedding
  storage key and active-profile backfill slice.
- Staged source version `0.5.155` for the merged embedding provider contract
  and failure lifecycle maintenance line.
- Staged source version `0.5.154` for the failure lifecycle maintenance
  feature: classify transient vs permanent failures, auto-requeue bounded
  transient extraction/replay/job failures, archive aged permanent/exhausted
  failures into history with an explicit `cleanup --archived-failures` purge
  path, and expose actionable-vs-archived failure stats in `status`/`doctor`.
- Staged source version `0.5.153`: batch session ingestion (`remem
  ingest-sessions` with per-file cursors and multi-root discovery) and raw
  time-window / session-listing queries (GH720 Phase 1, #722 #723).

### Fixed
- Staged source version `0.5.161` for the GH-717 post-merge semantic dedup
  follow-up: preserve numeric observation value changes, keep observation facts
  with narratives, and propagate preference API failures when fallback is off.
- Mapped memory-candidate extraction outputs that copy observation types
  (`feature`, `refactor`, `change`) back into the canonical candidate memory
  vocabulary instead of failing the whole extraction batch.
- Staged source version `0.5.125` without pointing plugin runtime downloads at
  an unpublished GitHub Release. The committed runtime manifest now stays local
  until the release workflow uploads checked assets.
- Hardened macOS ARM installer handling so ad-hoc codesigning failures are not
  silently ignored.

### Changed
- Added repository public-surface and file-size guardrails for release
  readiness.
- Added the `Auto Release` workflow so a passing `main` CI run tags staged
  source versions and lets the existing release workflow publish the assets.
- Staged source version `0.5.126` for the current-memory contract gates.
- Staged source version `0.5.127` for coding-bench contract artifacts.
- Staged source version `0.5.128` for workstream identity continuity.
- Staged source version `0.5.129` for usage feedback shadow ranking.
- Staged source version `0.5.130` for preference semantic-dedup calibration.
- Staged source version `0.5.131` for the coding-agent benchmark runner.
- Staged source version `0.5.132` for randomized coding-benchmark run order.
- Staged source version `0.5.133` for the public benchmark artifact verifier.
- Staged source version `0.5.134` for the remem-native memory benchmark suite.
- Staged source version `0.5.135` for the adversarial memory policy benchmark
  suite.
- Staged source version `0.5.136` for memory benchmark write-vs-retrieval
  diagnostics and baseline adapters.
- Staged source version `0.5.137` for the issue385-v1 coding benchmark task
  pack and `bench coding` dry-run alias.
- Staged source version `0.5.138` for coding-benchmark memory attribution and
  fixed failure taxonomy.
- Staged source version `0.5.139` for the directional public benchmark baseline
  report generator and checked-in baseline report.
- Staged source version `0.5.140` for preference semantic-dedup follow-ups:
  extraction source reduction, render-time cleanup, and merge cleanup clustering.
- Staged source version `0.5.141` for automatic release dispatch after
  bot-created release tags.
- Staged source version `0.5.142` for memory-candidate observation-type
  normalization.
- Staged source version `0.5.143` for review-gated temporal fact diagnostics.
- Staged source version `0.5.144` for summary promotion shadow-gate telemetry.
- Staged source version `0.5.145` for deterministic capacity eval scale curves.
- Staged source version `0.5.146` for associative multi-hop fixture headroom.
- Staged source version `0.5.147` for summary promotion enforce mode.
- Staged source version `0.5.148` for cross-process log rotation hardening.
- Staged source version `0.5.149` for foreground status schema convergence.
- Staged source version `0.5.150` for capacity degradation eval gates.
- Staged source version `0.5.151` for prefix-cache-stable context rendering.
- Staged source version `0.5.152` for Codex SessionStart context visibility.
- Staged source version `0.5.153` for the local embedding provider contract.
- Updated extraction-eval candidate prompt fingerprints for the
  memory-candidate type-vocabulary prompt change.

## [0.5.109] - 2026-06-20

### Added
- Documented the full native web API surface for remem-web, including
  capabilities, canonical memory browse/detail, stats, graph, candidate list,
  and candidate review endpoints.
- Added a local native API smoke test for the `remem api` read-model endpoints
  under bearer-token auth. This is the release-note entry for the planned
  `remem 0.5.109` web API surface; installed-binary docs should point users at
  it only after the `v0.5.109` tag and GitHub Release exist.
- After `v0.5.109` is published, remem-web should require `remem >= 0.5.109` for
  `/api/v1/capabilities.features.stats`, `memory_list`, `memory_detail`,
  `candidate_rows`, `candidate_review`, and `graph`; older clients can keep
  using `/api/v1/memory?id=` and `/api/v1/memories/list` compatibility paths.

## [0.5.104] - 2026-06-20

### Added
- Added current-state queries over `memory_state_keys` for CLI and MCP callers,
  including compact history, conflict, edge-evidence, and as-of-time output.
- Added human-editable markdown memory export and reindex import, including
  archived state, temporal facts, and current-state edge metadata.
- Added deterministic failure-trajectory lesson feeding from raw transcripts:
  repeated failed-fix evidence plus an explicit lesson now records an
  idempotent `failure` lesson outcome before summary short-circuits.

### Fixed
- Fixed current-state as-of history so mutable historical memory rows updated
  after the requested cutoff are not shown as if they were known then.
- Fixed graph-candidate review follow-ups so graph extraction only prompts for
  promotable edge types, deferred graph candidates stay visible in status, and
  graph tasks do not wait on memory tasks that already covered their range.
- Fixed markdown reindex restores so stale source hashes, cross-store
  provenance ids, older current-state slots, cross-memory fact supersession,
  and memory-edge remapping do not corrupt restored memory state.

### Changed
- Changed the npm wrapper package scope to `@remem-ai/remem` for the branded
  remem npm distribution.
- Added phase-0 extraction cursor integrity checks, model-provided confidence
  handling, and promotion metrics for extraction review workflows.
- Reframed project metadata and README docs around Claude Code and Codex as
  first-class hosts, including a Codex setup section and distribution channel
  guidance.
- Added Homebrew install docs and prepared an npm wrapper package for future
  npm publishing.

## [0.4.5] - 2026-05-26

### Fixed
- Updated the remaining GitHub Release action to a Node.js 24-compatible
  version.
- Updated Codex hook feature flag installation to use `[features].hooks` and
  remove the deprecated `[features].codex_hooks` alias.

## [0.4.4] - 2026-05-26

### Added
- Added release-binary installation docs for pinned versions, custom install
  directories, manual GitHub Release downloads, and binary-only installs.
- Added release asset checksums to future GitHub Releases.

### Fixed
- Updated GitHub release workflow artifact actions to Node.js 24-compatible
  versions.
- Fixed `remem install` binary path resolution so hooks and MCP use the current
  binary path or `REMEM_INSTALL_BINARY`, instead of always writing
  `~/.local/bin/remem`.

## [0.4.3] - 2026-05-26

### Added
- Added Codex context injection gating for SessionStart hooks: first injection
  emits full context, duplicate same-session context suppresses empty stdout,
  and changed context emits compact delta output.

### Fixed
- Fixed context gate fallback behavior so missing trusted session identity fails
  open, fallback cwd keys are canonicalized, and expired transcript-only fallback
  cooldowns re-emit full context instead of compact delta.
- Fixed context hash normalization for generated debug traces and stats footer
  totals so unchanged context is not repeatedly injected.
- Fixed migration dry-run validation to run post-migration hooks against a
  faithful on-disk backup clone while preserving owner-only temp permissions.
- Fixed backup import handling for malformed `topic_key` values and improved
  empty CLI search diagnostics.

## [0.4.2] - 2026-05-16

### Fixed
- Fixed Codex usage accounting to parse the current `codex exec --json`
  `turn.completed.usage` event instead of trying to match a run marker in
  ephemeral session logs. New Codex-backed rows now record `usage_source =
  codex_log` with cache and reasoning token breakdowns.

### Docs
- Updated usage accounting docs to describe the `codex exec --json` source for
  exact Codex token counts.

## [0.4.1] - 2026-05-16

### Packaging
- Bumped the crate and binary version to `0.4.1` for the post-`0.4.0`
  maintenance release.

## [0.4.0] - 2026-05-16

### Added
- Added `remem usage` for daily and weekly AI token/cost reporting.
- Added `ai_usage_events` token breakdown fields for input, output, reasoning,
  cache creation, cache read, raw input/output, usage source, and pricing source.
- Added Codex session JSONL token accounting keyed by a per-run remem id.
- Added historical usage repricing migration for older zero-cost rows.
- Added CLI search parity with the canonical memory service, including
  `--offset`, `--branch`, `--include-stale`, `--multi-hop`, and `--type` as a
  `--memory-type` alias.
- Added raw archive fallback previews and `has_more` guidance to CLI search.
- Added `--dry-run` previews for `pending retry-failed` and
  `pending purge-failed`.

### Changed
- Defaulted remem's Codex summarization model to `gpt-5.2`; set
  `REMEM_CODEX_MODEL=auto` to use the Codex CLI default.
- Updated model pricing to include current cache/reasoning-aware OpenAI and
  Anthropic price families.
- Serialized schema migrations with `BEGIN IMMEDIATE` to avoid concurrent
  migration races.
- Preserved the context stats footer when context output is truncated and the
  footer fits within the configured character budget.
- Propagated branch, memory-type, stale-state, and offset filters through
  multi-hop search expansion.

### Docs
- Documented usage reporting, precision levels, pricing overrides, and the
  `gpt-5.2` Codex default in English/Chinese README and architecture docs.
- Documented filtered multi-hop CLI search and pending dry-run operations in
  English and Chinese README files.

## [0.3.8] - 2026-04-03

### Packaging
- Excluded local-only artifacts from published package: `eval/local/results/` and `plan/`.
- Published `remem-ai` v0.3.8 to crates.io.

### Docs
- Fixed Cargo install command to `cargo install remem-ai --bin remem` in English and Chinese README files.

## [0.3.5] - 2026-03-26

### Packaging
- Switched SQLCipher build to `rusqlite` feature `bundled-sqlcipher-vendored-openssl`, so release builds no longer depend on runner-provided ARM64 OpenSSL packages.
- Simplified ARM64 release job back to `gcc-aarch64-linux-gnu` linker setup only.

## [0.3.4] - 2026-03-26

### Packaging
- Fixed ARM64 Linux toolchain install on GitHub Ubuntu runners by switching from multi-arch `libssl-dev:arm64` to cross package `libssl-dev-arm64-cross`.
- Updated ARM64 OpenSSL include/lib env paths (`/usr/aarch64-linux-gnu/...`) to match cross toolchain layout.

## [0.3.3] - 2026-03-26

### Packaging
- Fixed GitHub Release ARM64 Linux cross-compilation for SQLCipher by installing ARM64 OpenSSL toolchain (`libssl-dev:arm64`) and setting target-specific include/lib env vars in `release.yml`.
- Kept `reqwest` on `rustls-tls` to avoid unnecessary `native-tls` OpenSSL coupling in release builds.

## [0.3.2] - 2026-03-26

### Packaging
- Switched `reqwest` to `rustls-tls` (disabled default features) to remove `native-tls`/OpenSSL cross-build dependency.
- Fixed Linux ARM64 release build path in GitHub Actions by avoiding target OpenSSL toolchain requirement.

## [0.3.1] - 2026-03-26

### Architecture
- Introduced canonical `ProjectId` normalization and removed ad-hoc project matching paths.
- Added `MemoryService` to unify save/search behavior across MCP and REST API.
- Added `pending_admin` module and CLI commands for failed pending operations.

### Reliability
- Replaced destructive pending deletion on flush errors with recoverable pending state machine:
  `pending` / `processing` / `failed` plus retry metadata.
- Added DB migration to schema v13 for pending retry/failure fields and indexes.

### API / UX
- Unified memory write contract (`text`, `title`, `project`, `scope`, `memory_type`, etc.) for MCP and REST.
- Updated README command/API examples for failed pending inspection and retry.

## [0.3.0] - 2026-03-24

### Search
- **4-channel RRF fusion**: FTS5 + Entity Index + Temporal + LIKE, merged via Reciprocal Rank Fusion
- **Entity index**: Rule-based entity extraction (1600+ unique entities), `remem backfill-entities`
- **Temporal retrieval**: Parse "yesterday"/"上周"/"3 days ago" into time-range filters
- **OR semantics**: Multi-token FTS5 queries match ANY token (was AND)
- **Synonym expansion**: 50+ Chinese↔English term mappings (`query_expand.rs`)
- **Title-weighted BM25**: `bm25(fts, 10.0, 1.0)` — title matches weighted 10x
- **Hybrid routing**: Long tokens → FTS5, short tokens → LIKE, merged with dedup

### CLI
- `remem doctor` — 6-point system health check
- `remem search <query>` — Search memories from terminal
- `remem show <id>` — View memory details
- `remem eval` — Run search quality benchmark against golden dataset
- `remem backfill-entities` — Populate entity index from existing memories
- `remem encrypt` — Encrypt database with SQLCipher
- `remem api --port` — Start REST API server

### API
- REST API server (Axum) with 4 endpoints: search, get, save, status
- CORS support for browser-based integrations
- Binds `127.0.0.1` only

### Security
- SQLCipher encryption at rest (`bundled-sqlcipher`)
- Data directory permissions `0700`, log files `0600`
- Key file `~/.remem/.key` with `0600` permissions

### Architecture
- `ToolAdapter` trait for multi-tool support (Claude Code, future: Codex/Cursor)
- Split `memory.rs` (1308→553 lines) into `memory.rs` + `memory_search.rs` + `memory_promote.rs`
- Fine-grained memory promotion: multi-item decisions/learned split into individual memories
- SQL-layer project suffix-match filter (was post-filter)
- Content-derived titles (was request-prefix truncation)
- Search-friendly summary prompt rules

### Testing
- 128 tests (87 unit + 14 benchmark + 14 promote + 13 integration)
- Benchmark suite: 9 evaluation dimensions, 14 automated tests
- Golden dataset v1.1: 30 real-world queries, 24 with calibrated ground truth
- IR metrics: NDCG, MRR, Precision@K, Recall@K, Hit@K

### Search Quality (1001 real memories, 30 queries)
- MRR: 0.858
- Precision@5: 0.460
- Recall@5: 0.628
- Hit Rate@5: 1.000
- CJK dictionary segmentation: "数据库加密" → "数据库"+"加密" → database+encrypt
- 90+ Chinese↔English synonym mappings
- Core-token LIKE channel (CJK-segmented, no synonym noise)

## [0.2.0] - 2026-03-23

Initial public release with MCP server, hooks integration, session summaries, preferences, and FTS5 search.

## 2026-03-03

### Added
- Added a persistent job queue in SQLite (`jobs` table) with lease/retry/failure states.
- Added worker execution path (`remem worker`) for queued observation/summary/compress jobs.
- Added read-only Bash filtering coverage for `grep`/`rg`/`find`/`git grep` and polling-style `curl` commands.
- Added unit tests for Bash filter behavior to ensure read-only commands are skipped while mutating commands are retained.

### Changed
- Changed `summarize` hook behavior to enqueue jobs and return quickly, then trigger worker processing.
- Changed flush execution path to use `observe_flush` module and worker-driven orchestration.
- Updated install/runtime wiring to include new worker/queue flow.
- Tuned observation capture logic to reduce low-value shell noise in pending queue.
