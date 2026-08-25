# GH969 Product Contract — Stabilization And Surface Governance

Status: Current contract; activation, surface, and dependency-direction guards implemented; Issue: #969

Last reconciled against `origin/main`: 2026-08-25 (`5b98e80d`)

## Purpose

remem optimizes for memory quality. Stabilization must make high-capability
memory behavior safer and easier to verify; it must not remove automatic
capture, LLM extraction, governed promotion, graph evidence, or evaluated
retrieval merely to reduce cost or code size.

This contract is the canonical cross-cutting acceptance contract for #969. It
does not replace the narrower current contracts linked below. Where a
surface-specific contract is stricter, that contract wins. Where historical
root `specs/GH*/` packets disagree with current code or `docs/specs/`, current
code plus the current contract wins.

## User Outcomes

1. No unreviewed or untrusted generated content silently becomes active
   memory.
2. A memory, relation, or recovery action cannot cross its project or owner
   boundary and suppress an in-scope claim.
3. Production claims describe the production path actually executed, with
   persisted evidence rather than fixture-derived counters.
4. Experimental surfaces are visibly experimental, have an owner and a
   decision date, and cannot become default behavior without evidence.
5. Recovery behavior preserves historical data without becoming a second
   production writer.
6. Contributors can identify the active data flow, module direction, evidence
   gate, and rollback path before changing shared runtime behavior.

## Non-Goals

- Removing automatic hook capture or LLM extraction to save cost.
- Replacing remem with a minimal SQLite/FTS memory store.
- Splitting the package into crates before stable dependency boundaries exist.
- Treating module count, source size, or lack of direct CLI use as sufficient
  reason to remove a quality-producing capability.
- Claiming that a spec, synthetic fixture, dry run, or closed GitHub issue is
  proof that runtime acceptance is complete.
- Making the Context Bundle MCP schema, Retrieval Router, CurrentTruth v2, or
  public benchmark claims stable merely by documenting them here.

## Current Reconciliation

The original #969 audit was correct for its v0.6.27 source snapshot. The
following repository-local findings now have landed implementation evidence:

| Finding | Current evidence |
|---|---|
| Dream generated output bypassed review | Governed Dream quarantine/promotion and historical backfill are covered by `memory-poisoning-defense/`; #987 and #990 landed. |
| External candidates deduplicated across project/owner boundaries | Project and ownership identity participates in external-candidate identity; #987 landed. |
| Adversarial eval did not execute production governance | The production-path database harness and persisted-state diagnostics are covered by `memory-poisoning-defense/`; #991/#998 landed. |
| Legacy `events` dual-write could be non-idempotent | Canonical capture and the compatibility projection are transactionally linked and idempotent under `legacy-observation-retirement/`; #992 landed. |
| Context Bundle and SessionStart had parallel output semantics | The default `bundle` render mode consumes the compiled Context Bundle and persists a hash-bound audit under `GH932/`. `REMEM_CONTEXT_BUNDLE_RENDER_MODE=legacy` rolls back bundle relevance compilation/audit only; CurrentTruth still governs Core selection and abstentions. |
| CurrentTruth was diagnostic-only | The safe v1 projection is used by the default Context Bundle/SessionStart path; #1019/#1029/#1032 and the #1037/#1038 cross-subject isolation shipped in v0.6.81. Breaking v2 work remains separate under #933. |
| Retrieval Router was a plan-only concept | Explicit routed MCP search can apply plan effects, but complete plan-controlled execution and default-on evidence remain pending under `GH934/`. |

These landed slices do not close the epic. The #1040 slice implements the
single active-memory activation boundary and bypass guard for v0.6.82. The
#1044 slice adds the reviewed dependency-direction baseline and
no-expansion guard. Remaining material work is the unified decision/evidence
matrix, outcome scorecard, and final architecture/spec synchronization defined
by this contract.

## Active-Memory Safety Boundary

An **activation** is any insert or update that leaves a curated memory row
eligible as `status='active'`. Every activation must pass one logical boundary,
even if compatibility wrappers remain during migration.

The boundary must require and atomically persist:

- canonical project, presence-sensitive branch, scope, owner, and optional
  target identity;
- source operation and actor;
- source trust class;
- candidate, supplemental-save caller evidence, governed-import, or exact
  recovery provenance;
- evidence identity when the source is derived;
- generated-surface and source-surface poisoning verdicts as applicable;
- the exact supersede set and policy;
- an operation/audit identity that makes retries idempotent.

Allowed activation routes are:

| Route | Minimum rule |
|---|---|
| Supplemental manual save | Preserve MCP and REST direct save for both people and agents, scan the exact payload, and persist the real caller plus evidence-derived trust. An agent call or shared API token is not user attestation; any future human attestation surface is separately reviewed. A semantic no-op must not weaken the existing row's trust or acknowledgement evidence, and an idempotent replay returns the original durable claim and local-copy outcomes rather than synthesizing a new response or timestamped path. Agent-facing save schemas do not expose human acknowledgement; quarantined content uses the reviewed candidate-governance surface. |
| Governed candidate promotion | Candidate identity, evidence, trust, route, review decision, and supersede policy are mandatory. Auto-promotion remains limited by the current candidate contracts. |
| Generated consolidation | Never activates directly. Clean output enters the same governed promotion boundary; risky output remains quarantined and cannot supersede active rows. |
| Import or host-native memory | Enters review candidates unless a narrower current contract proves an equivalent digest-bound governed route. The current safe-add project pack import is such a route and remains supported. |
| Migration or recovery | May restore an exact previously active row only with a versioned plan, immutable evidence binding, explicit acknowledgement where required, and idempotent apply. Backup evidence must cover the same consistent database snapshot that is read, and restored acknowledgement metadata must be complete and match the restored payload. It cannot invent a new active claim. |

Direct SQL, test helpers, migration DDL, and compatibility writers are not new
production activation routes. The implementation guard may allow them only in
explicitly enumerated files, and every allowance needs a reason and owner.

## Trust Taxonomy

The current ordered source classes remain:

1. `external_content`
2. `pack`
3. `local_tool_output`
4. `repo_file`
5. `user_prompt`

Trust is evidence provenance, not a truth score. Derived content inherits the
least-trusted material evidence unless a stricter surface contract applies.
Unknown or malformed trust fails closed. `external_content` cannot auto-promote.
Model generation never raises source trust, and generated output is scanned as
a separate surface. Review may authorize a specific candidate and supersede
set; it does not retroactively relabel the source as trusted.

## Lifecycle Vocabulary

Every major surface has exactly one lifecycle status per exposed entry point:

| Status | Meaning | Exit rule |
|---|---|---|
| `staged` | Landed and intended for a supported default runtime path on `main`, but not yet present in the latest published release; it is explicitly not `production`. | Becomes `production` only after release verification; rollback/removal before publication must update the inventory. |
| `production` | Present in a verified published release, used by a supported runtime path, and not classified as experimental, recovery-only, or deprecated. | Replacement or retirement requires migration, user-visible docs, and release evidence. |
| `experimental` | Explicitly classified opt-in or separately labelled API, whether or not its code has been published; it is not required for normal operation and is not `production`. | Integrate after the default-on gate, continue with a new decision date, or remove. |
| `recovery-only` | Explicitly classified reader/replayer for historical state under bounded rules; it accepts no new normal writes and is not `production` even when a production worker invokes its bounded drain. | Retire only after measured residual state is zero for the contract's retention window. |
| `deprecated` | Explicitly classified compatibility surface with a visible replacement and removal plan; it is supported only for that compatibility window, not `production`. | Remove only at the declared compatibility boundary. |
| `spec-only` | Contract or planned surface with no runtime implementation claim and therefore no overlap with any implemented status. | Becomes staged, experimental, or production only through an implementation PR with tests and evidence. |

The manifest records the status explicitly; the guard does not infer multiple
statuses from adjectives such as “released” or “opt-in”. It instead verifies
the mutually exclusive invariants above and rejects contradictory metadata.
Published-release verification uses the remote tag, non-draft release, assets,
and the release's distributed manifest. The source-tree
`plugins/remem/runtimes/remem-releases.json` is auto-release staging input and
does not override an already published, hash-matching release artifact.

Internal modules are not automatically public promises. Public APIs must carry
an explicit lifecycle label; unlabeled callable surfaces fail the inventory
guard.

## Canonical Surface Inventory

This table is the canonical human-readable inventory. Its expanded
machine-readable form lives at `surface-manifest.json`; the repository-owned
public-surface check validates it against host-compiled, signature-fingerprinted
Rust exports, a conservative inventory that recursively follows public contents
of platform-cfg modules and their public associated items, the normalized
input/output schemas returned by the served MCP `tools/list` response, and the
real REST, Clap, Cargo-feature, and offline-harness registration roots. REST
method+path identities include a normalized catalog of request/response serde
declarations and constructed JSON shapes; unclassified Axum service or fallback
registrations fail closed. Default Cargo features require explicit row mappings.

The grouped Rust/MCP/REST/CLI rows below are exhaustive discovery rules, not
sample entries. The machine manifest expands them to one record per reachable
Rust export, registered tool, method+path, and Clap command. Offline harness
rows use a separate explicit entry type whose checked roots, executable scripts,
schemas, and fixture/data inventories are resolved from the repository rather
than from the Rust surface graph. An entry inherits the group status unless a
more specific row below overrides it; an undiscovered or multiply classified
public entry fails the guard.

The manifest also carries the reviewed `published_surfaces` baseline and its
release version. A newly discovered entry in a production group is generated as
`staged` until an explicit post-release `--promote-published <version>`
regeneration verifies the exact non-draft GitHub release, its distributed
assets, and its `surface-manifest.json` before advancing the baseline from that
artifact. Published removals and signature changes fail closed unless the exact
old identity is first added to the append-only `retired_surfaces` audit ledger
in the reviewed removal PR. That review must point to the owning migration,
user-visible compatibility notice, and removal-boundary evidence; an entry in
the ledger is valid only after the surface is no longer reachable. Every expanded
record copies all eight canonical table columns, so
changing owner, caller/default, evidence, compatibility, or next-decision text
requires the same reviewed manifest update as changing status.

| Inventory row | Entry point | Owner | Status | Real caller / default | Evidence | Compatibility | Next decision |
|---|---|---|---|---|---|---|---|
| `rust-library` | Exported Rust library surface | `src/lib.rs` and reachable public modules/re-exports | `production` | Every host-resolved exported module/symbol and public associated item with a normalized signature fingerprint, plus recursively discovered public contents of platform-cfg modules, except entries explicitly overridden below | Public API tests plus exhaustive export/signature discovery | Published exports follow SemVer; new or signature-changed symbols remain staged until release verification | Continuous; review on export-set or signature change |
| `mcp-production` | MCP production tool set | `mcp/server/`, `mcp/server/tests/tool_metadata.rs` | `production` | All registered tools except the explicit experimental `context_bundle` entry: `current_state`, `search`, `recall_user_context`, `timeline`, `get_observations`, `lookup_commit`, `commits_for_session`, `save_memory`, `govern_memory`, `timeline_report`, `workstreams`, `update_workstream`, `search_raw`, `list_raw_sessions` | Registry completeness, metadata, schema, served-wire, and legacy-text tests | Tool names and stable legacy response contracts remain supported; routed `search` parameters are overridden below | Continuous; review on registry change |
| `rest-api` | REST `/api/v1` method+path set | `api/server.rs`, `api/handlers/` | `production` | Every method+path reachable from `build_router`, including composed routers, Axum's implicit HEAD for GET, and memory save/archive/restore and candidate review | `tests/api_public.rs` and handler tests | Bearer transport auth is not human provenance; route/schema changes follow public API compatibility rules | Continuous; review on router change |
| `cli-production` | CLI command tree | `cli/types.rs`, `cli/dispatch.rs` | `production` | Every default-feature compiled Clap command/subcommand and alias, with eval/report, plan, and recovery operations overridden by their specific inventory/spec rows | CLI parser/dispatch tests and command-specific contracts | Existing supported commands and aliases remain compatible; feature-gated absence is recorded in the expanded manifest | Continuous; review on command-tree change |
| `sessionstart-context-bundle` | SessionStart Context Bundle compiler and audit | `context/`, `context_bundle/` | `production` | Host SessionStart in default `bundle` mode; `legacy` rolls back bundle relevance/audit but retains CurrentTruth-governed Core output | GH932 tests, SessionStart audits, coding-bench plan/audit evidence | Rendered SessionStart behavior and persisted audit schema require migration-aware change | Continuous; review on schema/policy bump |
| `mcp-context-bundle` | MCP `context_bundle` v1 | `mcp/`, `context_bundle/` | `experimental` | Explicit MCP caller; never required by SessionStart clients | Closed schema/served-wire tests | Versioned JSON, explicitly experimental; no stability claim beyond declared schema version | 2026-11-30 |
| `rust-context-bundle` | `remem::context_bundle` exported API | `context_bundle/` | `experimental` | Explicit Rust caller; default SessionStart uses an internal adapter rather than making this API stable | GH932 schema/policy tests | Versioned but explicitly experimental | 2026-11-30 |
| `currenttruth-v1` | CurrentTruth v1 context projection | `truth/`, `context_bundle/current_truth.rs` | `production` | Default Context Bundle/SessionStart path since v0.6.81, including legacy relevance mode's Core rendering | GH933 v1 tests, production activation regressions, #1038 relation isolation, published v0.6.81 tag/assets | No CurrentTruth-specific v1 rollback exists; the broader old-path rollback remains pending Phase B acceptance under #933 | Continuous; review under #933 |
| `doctor-truth` | `remem doctor truth` | `doctor/truth.rs`, `truth/` | `production` | Explicit diagnostic | Projection/rehearsal diagnostics | Read-only diagnostic output may evolve additively within v1 | Review with GH933 v2 |
| `retrieval-router-plan` | Retrieval Router plan compiler and `context-plan` | `retrieval_router/`, `cli/` | `experimental` | Explicit diagnostic/plan caller | Determinism and schema tests | Versioned plan schema; no default execution promise | 2026-11-30 under GH934 |
| `rust-retrieval-router` | `remem::retrieval_router` exported API | `retrieval_router/` | `experimental` | Explicit Rust caller; not a stable default executor | GH934 plan determinism tests | Versioned but explicitly experimental | 2026-11-30 under GH934 |
| `routed-search-parameters` | Routed MCP `search` parameters | `mcp/server/search_routing.rs`, `retrieval_router/` | `experimental` | Explicit intent/role/risk/budget only; ordinary search remains supported | Applied-effects tests; incomplete full-executor ablation | Optional parameters must not change legacy requests | 2026-11-30 under GH934 |
| `graph-edges` | Trusted literal `graph_edges` retrieval channel | `retrieval/graph/`, `memory/graph_contract.rs` | `production` | Weighted retrieval search; default graph weight is non-zero | Associative and graph-decision gates, scope/leak/latency tests | Rollback is graph weight zero; graph provenance/schema remain governed | Continuous; rerun decision gate on policy/weight change |
| `entity-bfs` | Entity BFS diagnostic/eval arm | `retrieval/entity/`, `eval/graph_decision.rs` | `experimental` | Eval/diagnostic comparison, not the literal-graph ship decision | Informational graph-decision arm | No default-on promise | 2026-11-30 |
| `local-onnx` | `local-onnx` embedding provider | `retrieval/embedding/`, Cargo feature | `production` | Feature is built by default; activation remains conditional because `Auto` selects only verified local artifacts | `local-semantic-embedding/` reports and provider tests | Provider/model identity is persisted; operator/provider rollback remains available | Review on model/default-selection change |
| `deterministic-eval` | Deterministic eval gates | `eval/`, `eval/gates.rs` | `production` | CI/preflight engineering gate and explicit feature-gated CLI; overrides the general CLI row | Golden, capacity, SessionStart and security gates | Threshold changes require same-PR baseline rationale | Continuous |
| `coding-public-benchmarks` | Coding/public benchmark reports | `eval/coding_bench/`, `eval/public/` | `experimental` | Evidence remains non-claim-bearing until claim gates pass; explicit runner/report commands only | GH931 and `public-memory-benchmark/` | Artifacts are immutable/hash-bound; draft rows remain directional | 2026-11-30; external authority may justify an explicit dated continuation, never an undated exemption |
| `legacy-pending` | Legacy `pending_observations` replay/admin | `db/pending/admin/`, worker idle bridge | `recovery-only` | Existing residual rows via the declared idle-only automatic worker drain or explicit CLI/admin operations; no new normal writer | `legacy-observation-retirement/` and failure-lifecycle tests | Exact row identity, bounded replay, visible failures | Retire only after contractually measured zero residual state |
| `legacy-events` | Legacy `events` compatibility projection | `db/capture.rs`, legacy query surfaces | `deprecated` | Transactional compatibility projection from canonical capture; no independent source of truth | #992 idempotency/failure regressions | Preserve readers until a separately announced removal boundary | 2026-11-30 inventory review |
| `historical-summary` | Historical Summary jobs/writers | failure lifecycle compatibility | `recovery-only` | New dispatch is rejected; retained only for explicit diagnostics/history | Legacy retirement tests | Never revive as a production writer | Remove only with migration evidence |
| `currenttruth-v2` | CurrentTruth v2 native writer/cutover | `docs/specs/GH933/` | `spec-only` | No production caller | Migration, rehearsal, rollout contracts only | Requires approved breaking cutover and rollback | 2026-11-30 under #933 |
| `cross-host-harness` | Cross-host offline harness | `eval/cross-host/`, `docs/specs/GH935/` | `experimental` | Explicit offline schema/scanner/dry-run commands; no live execution claim | GH935 v1 infrastructure tests | Artifact schema is versioned; dry-run output is not a result | 2026-11-30 |
| `cross-host-completion` | Cross-host benchmark completion | `docs/specs/GH935/` | `spec-only` | Completion is unimplemented beyond the experimental offline harness above | No live claim-bearing matrix | No public result until prerequisites and official runs exist | 2026-11-30; continue only with dated GH931/user-identity dependency evidence |

Adding a major surface or changing a row's status requires updating this
contract or a machine-readable manifest linked to it in the same PR.
Experimental implementation rows also carry a fingerprinted inventory of every
production source file referencing their activation symbols. New callers or
changes inside an existing caller require an explicit manifest regeneration and
review of whether the surface has entered a default path. Offline harness
categories are checked independently; a script cannot satisfy the executable
contract by being relabelled as a document.

## Decision Gates

Passing one gate never implies passing a later gate.

| Decision | Required evidence | Does not authorize |
|---|---|---|
| Merge | Focused regression tests, repository preflight appropriate to scope, current spec alignment, no new silent degradation, and reviewed migration/rollback for risky changes | Release, default-on behavior, or public claims |
| Release | Exact-head merge evidence, version synchronization, supported-platform CI/smoke results, migration compatibility, upgrade/rollback notes, and release authorization | Experimental default flips or stronger public quality claims |
| Default-on | Same-head baseline vs enhanced ablation, pre-registered quality and harm metrics, latency/resource budgets, failure-mode tests, a user-visible or operator rollback, and no scope/trust regression | Public comparative or superiority wording |
| Public claim | Immutable claim-bearing artifact, production-path execution, complete condition matrix, independent scoring/verification, declared exclusions, stop-loss result, and any external authority required by the benchmark contract | Broader claims than the sealed artifact supports |

## Product Evidence And Stop-Loss

Advanced retrieval or memory behavior must pre-register capability-specific
thresholds before running the comparison. At minimum, artifacts report:

- correct-memory/evidence recall and task completion;
- repeated-explanation rate;
- helpful-memory rate;
- wrong, stale, poisoned, and cross-project/owner injection rate;
- abstention and missing-evidence rate;
- foreground latency and maintenance cost;
- rollback/default state and exact implementation SHA.

No universal recall delta is frozen here because channels solve different
tasks. The narrower spec must freeze its threshold before observing the result.
Security boundary regressions are zero-tolerance. If an experimental capability
misses its pre-registered benefit threshold or exceeds a side-effect threshold
in two consecutive accepted decision artifacts, it must be gated off, rolled
back, removed, or receive an explicit new experiment hypothesis and decision
date; it cannot remain indefinitely “partial”.

## Migration And Rollback Principles

- Additive schema first; readers tolerate the staged state before writers use
  it.
- Backfills use plan/apply, immutable input identity, idempotent replay, and
  visible partial failure. Destructive or trust-changing backfills need a
  backup and rehearsal.
- Production changes retain a tested rollback path until the new path has
  passed its observation window. Rollback must not discard new user data.
- Compatibility paths are read/recovery adapters, not a second source of
  truth.
- Unknown trust, scope, owner, schema, or audit identity fails closed and is
  visible to the user or operator.
- Version bumps are required when binary/runtime behavior changes; spec-only
  work does not create a fake release bump.

## Acceptance Criteria

This spec-only slice is accepted when:

- `docs/specs/GH969/PRODUCT.md`, `TECH.md`, and the specs index land together;
- the PR uses `Refs #969` and does not close the implementation epic;
- current implementation claims in the inventory are verified against code and
  narrower current specs;
- the remaining implementation slices are represented by focused child issues
  after the spec is accepted.

The #969 epic is complete only when:

- the active-memory boundary and its bypass guard are implemented;
- a machine-readable surface inventory and status-consistency guard are active;
- the dependency-direction guard prevents expansion of the current cyclic
  component and the accepted baseline shrinks;
- the ship matrix and user-outcome scorecard are executable, not only prose;
- architecture and current-spec declarations match production callers;
- every referenced experimental surface has reached an explicit integrate,
  continue-with-date, or remove decision;
- automatic capture, LLM extraction, memory quality, host compatibility, and
  visible failure behavior have not regressed.

## Related Current Contracts

- `memory-poisoning-defense/` — trust, quarantine, Dream, production security eval
- `legacy-observation-retirement/` — frozen/recovery legacy paths and events projection
- `GH932/` — Context Bundle and SessionStart integration
- `GH933/` — CurrentTruth v1 and pending v2 migration/cutover
- `GH934/` — Retrieval Router graduation evidence
- `GH931/` and `public-memory-benchmark/` — claim-bearing eval authority
- `local-semantic-embedding/` — local provider/default evidence
- `associative-multihop-fixtures/` — literal graph decision evidence
- `spec-lifecycle-governance/` — epic/spec/implementation PR lifecycle
