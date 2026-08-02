# GH933 Technical Contract — CurrentTruth Projection
Refs #933. `PRODUCT.md` defines product behavior. This file is the normative
Phase A v2 implementation contract.

## Status and Module Boundary

Truth v1 shipped in `remem-ai` 0.6.26/0.6.27; the Cargo target is `remem`, so
the public API is `remem::truth`.

The Phase A v2 projection call remains read-only. Exact historical routing
requires a narrow durable history substrate and its canonical writers:

```text
Cargo.toml/Cargo.lock + src/db/{core,sql_functions}.rs + all connection constructors
src/db/capture.rs (duplicate-event timestamp immutability + pure preview helper)
src/migrations/vNNN_current_truth_history_ledgers.sql + src/migrate/run.rs/backfill
src/memory/store/write.rs + src/memory/{operation,lifecycle}.rs + src/memory/service/{types,save,local_copy}.rs
API/MCP save adapters + memory_candidate/apply.rs + CLI import/Markdown/pack writers
src/memory/governance.rs + src/memory/scope_cleanup/{mutate,plan}.rs
src/doctor/** + route-mutating eval/test fixtures
src/truth.rs + src/truth/{adapter,lifecycle,projection,types}.rs + src/truth/tests/** + tests/truth_public_api.rs
```
No projection query may write/migrate/call external systems/change Context
Bundle. The foreground schema migration/backfill and atomic route/lifecycle
instrumentation below are the only additional writer scope; duplicate captures
also preserve row timestamps. Split `src/truth/tests.rs` before v2 tests; every
source file stays below 800 lines.

## Public v2 Types

All enums serialize with snake_case. `TruthScope` is internally tagged with
`scope_kind`. Optional fields serialize as explicit `null`.

```rust
pub const TRUTH_PROJECTION_VERSION: u32 = 2;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimSource {
    Memory,
    UserContextClaim,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct SubjectIdentity {
    pub source: ClaimSource,
    pub owner_scope: String,
    pub owner_key: String,
    pub memory_scope: Option<String>,
    pub kind: String,
    pub key: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "scope_kind", rename_all = "snake_case")]
pub enum TruthScope {
    Project {
        project: String,
        branch: Option<String>,
    },
    Owner {
        owner_scope: String,
        owner_key: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TruthQuery {
    pub scope: TruthScope,
    pub as_of_epoch: Option<i64>,
    pub subject: Option<SubjectIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionReplayability { Exact, CurrentSnapshotOnly }

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    CapturedEvent,
    SourceRef,
    SourceTrustClass,
    Observation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceIntegrity {
    Validated,
    Opaque,
    Quarantined,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvidenceView {
    pub evidence_ref: String,
    pub kind: EvidenceKind,
    pub source_ref: String,
    pub scope: TruthScope,
    pub lifecycle: Option<Lifecycle>,
    pub source_time_epoch: Option<i64>,
    pub knowledge_time_epoch: i64,
    pub trust: EvidenceTrust,
    pub integrity: EvidenceIntegrity,
    pub supporting_evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ClaimView {
    pub canonical_ref: String,
    pub subject: SubjectIdentity,
    pub statement: String,
    pub branch: Option<String>,
    pub lifecycle: Lifecycle,
    pub valid_from_epoch: Option<i64>,
    pub valid_to_epoch: Option<i64>,
    pub source_time_epoch: Option<i64>,
    pub knowledge_time_epoch: i64,
    pub evidence: Vec<EvidenceView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CurrentTruthView {
    pub subject: SubjectIdentity,
    pub claim: Option<ClaimView>,
    pub validity: ValidityState,
    pub evidence: Vec<EvidenceView>,
    pub supporting_relations: Vec<RelationView>,
    pub contradicting_relations: Vec<RelationView>,
    pub rejected: Vec<String>,
    pub conflicting_claims: Vec<ClaimView>,
    pub selected_reason: TruthSelectionReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CurrentTruthProjection {
    pub projection_version: u32,
    pub scope: TruthScope,
    pub requested_as_of_epoch: Option<i64>,
    pub reference_epoch: i64,
    pub replayability: ProjectionReplayability,
    pub truths: Vec<CurrentTruthView>,
    pub evidence_catalog: Vec<EvidenceView>,
}

pub fn project_current_truth(
    conn: &rusqlite::Connection,
    query: &TruthQuery,
) -> anyhow::Result<CurrentTruthProjection>;

pub fn project_user_claim_truth(
    conn: &rusqlite::Connection,
    owner_scope: &str,
    owner_key: &str,
    as_of_epoch: Option<i64>,
) -> anyhow::Result<CurrentTruthProjection>;
```

Existing `RelationView`, `Lifecycle`, `EvidenceTrust` and
`TruthSelectionReason` serde names remain stable. Project reads memory claims
plus its Observation catalog; Owner reads exact-owner memories + user claims
and has an empty catalog. Owner selectors require that owner. Project selectors
require a non-global Memory identity, but membership uses row routing + branch,
not `owner_key=project`; owner Q routed by `target_project=P` can match
Project(P). Incompatible selectors error. A compatible selector with no scoped
row returns `truths=[]`; `Unknown` requires a loaded identity with no survivor.
Selectors filter `truths`, never the Project catalog.

`project_current_truth` is normative and dispatches on `TruthQuery.scope`.
`project_user_claim_truth` selects UserContextClaim before loading, then uses
the shared Owner resolver. It never enumerates unrelated memories, memory-only
relations, Observation attachment or memory-only suppression. It loads only
bounded `user_claim`/`pattern` suppressions applicable to selected claims. Only
an explicit `preference_backfill` ref reads a memory; unrelated malformed owner
memory cannot affect the wrapper, but a malformed referenced memory can.
Exact-owner memories require the normative Owner query.
The v1 `load_memory_claim_groups`/`load_user_claim_groups` exports become
crate-private; README documents this migration.

`Some(t)` yields `Some(t)/t/Exact`; `None` samples once and is Exact unless a
no-proof binding/entity link makes it `CurrentSnapshotOnly`, not a replay key.
Every stage shares that epoch and snapshot: autocommit owns deferred BEGIN plus
terminal COMMIT/ROLLBACK; a caller transaction is reused, never nested/committed.

Stable output ordering:

- truths: derived lexicographic `SubjectIdentity` order;
- evidence: `(source_time_epoch, knowledge_time_epoch, evidence_ref)`,
  with `None` first;
- catalog: same key, de-duplicated by `evidence_ref`;
- relations: `(created_at_epoch, relation_ref)`;
- canonical ref lists: byte order.

Evidence field semantics:

| Kind | scope | lifecycle | source/knowledge time | integrity | supporting refs |
| --- | --- | --- | --- | --- | --- |
| CapturedEvent | canonical event Project, branch `None` | `None` | immutable first-capture reference/created; insertion | Validated | empty |
| SourceRef | containing user-claim Owner | `None` | claim-version valid-from-or-created; immutable provenance-binding epoch | Validated for resolved structured refs; Opaque only for manual/free-form refs | resolved nested canonical refs, or empty |
| SourceTrustClass | containing memory query scope/claim branch | `None` | memory reference/created; effective memory knowledge below | Validated | empty |
| Observation | canonical Observation Project/branch | `Some(observation lifecycle)` | Observation reference/created; Observation `created_at_epoch` | Validated or Quarantined | validated captured-event refs |

`SourceTrustClass` is diagnostic/cap provenance and is excluded from
`strongest_evidence`; the cap is applied separately. Captured events use branch
`None` because the canonical event schema has no branch field. The adapter must
not infer the current workspace branch. Opaque SourceRef means its JSON shape is
valid but the free-form pointer has no ledger integrity proof.

CapturedEvent `source_ref` remains the v1 `event_type/role` form (or
`event_type` when role is absent). SourceTrustClass `source_ref` is the
validated stored class. SourceRef uses canonical compact JSON for structured
objects and the stored source-kind label for an empty manual source.

Stable refs are `captured_event:<event-id>`, `memory_trust_class:<memory-id>`,
`observation:<observation-id>`, empty manual
`user_claim_source:<claim-id>:manual`, otherwise
`user_claim_source:<claim-id>:<path>`. A legacy top-level manual string is a
synthetic array at path `0`; arrays recursively canonicalize, exact-dedupe and
byte-sort before zero-based `:` paths (`0:0`), and wrappers point to children.

User-claim `source_kind` is a closed mapping:

| source_kind | cap / required provenance |
| --- | --- |
| `manual` | Verified; zero refs or only nonblank strings/`manual_cli`, all Opaque |
| `explicit_user_statement` | Verified cap; 1+ captured events, all first-party (`event_type=user_prompt_submit` or `role=user`), plus at most one session summary |
| `preference_backfill` | cap equals the referenced memory’s effective trust; requires exactly one valid memory ref |
| `inferred_from_behavior` | ModelGenerated; 1+ behavior captured events plus at most one session summary |
| `session_summary` | ModelGenerated; 1+ captured events plus at most one session summary |
| `speculative_inference` | ModelGenerated; 1+ captured events plus at most one session summary |
| `third_party_statement` | Untrusted; 1+ captured events plus at most one session summary |
| `user_context_candidate` | exactly one authoritative candidate wrapper; cap derives recursively |

`source_refs_json` must be an array except that legacy `manual` may be a
nonblank scalar string. Only `manual` may use an empty array, emitting one
Opaque `source_ref=manual`; every other kind needs a resolved ref and meets its
row above. Manual strings stay Opaque; other scalars/blanks error. Objects use:

The table is an exact allowlist: extra ref kinds/counts error. The five
terminal extraction kinds use the canonical writer’s captured-event +
optional-one-summary shape; direct summary-only provenance is rejected even
for `session_summary`. A candidate wrapper may instead recurse only through
the single-wrapper rule below.

| ref kind | exact fields and types | resolution, scope and time |
| --- | --- | --- |
| `captured_event` | `{"kind":"captured_event","id":<positive integer>}` | row must exist; source/knowledge epochs must be eligible; an enclosing candidate requires exact host/project/session provenance, while a direct ref uses the containing `repo` owner key as project anchor |
| `session_summary` | `{"kind":"session_summary","id":<positive integer>}` | recognized but unusable in Phase A under the read-only limitation below |
| `memory` | `{"kind":"memory","id":<positive integer>}` | row must exist, be time-eligible and have the same canonical owner as the containing claim; trust is that memory’s effective trust |
| `manual_cli` | `{"kind":"manual_cli","command":<nonblank string>}` | no row resolution; remains Opaque in the containing Owner scope |
| `user_context_candidate` | `{"kind":"user_context_candidate","candidate_id":<positive integer>,"source_kind":<string>,"source_refs":<array>}` | validate against the authoritative candidate and recursively resolve its refs |

No extra object fields are accepted. A project-bound direct ref on a non-repo
Owner has no persisted project anchor and is therefore an error. Missing,
foreign-scope, quarantined or malformed referents are contextual errors. A
referent after the immutable binding epoch is also an integrity error; a valid
bound referent after only the query cutoff is time-ineligible. Exact sibling
refs collapse after canonical compact JSON (or exact string bytes); the same
kind/ID with a different payload errors. Canonical/supporting refs are
byte-sorted.

A terminal ref binds at its enclosing candidate creation, or at the earliest
validated edit-chain version that introduced the exact kind/refs for a direct
ref. Later edits preserve them and never rebind provenance. Source/knowledge
must be `<=` binding/reference; event knowledge is insertion and memory uses
`effective_memory_knowledge_epoch`. Wrapper knowledge is validated application
and must be `<=` its containing binding (root-result creation at top level,
enclosing-candidate creation when nested). Comparisons are second-granular:
equality is eligible and absolute same-second ordering needs a Phase C sequence.
`explicit_user_statement` requires at least one captured ref and every captured
ref must be first-party even if canonically Verified.

A top-level `user_context_candidate` has one wrapper. Each candidate has
nonblank host/session/project; event/optional-summary refs exact-match its
`(host_id,project_id,session_row_id)`. Nested kind is one terminal kind with its
exact refs and no wrapper, or `user_context_candidate` with exactly one wrapper.
Candidate owner/kind/refs and application `<=` containing binding/reference
must validate. Its result ID names its own initial result: top-level is current/
ancestor; nested validates its own root/edit chain. A replacement root exact-
copies user/owner/type/key/text/confidence/sensitivity, wrapper and null validity
and shares creation/initial-update/last-confirmed with application. Descendants
preserve user/owner/confidence/source kind/refs and bind creation to edit;
no-op keeps the ordered exact match and its refs. Recursion repeats checks;
ancestry ID reuse cycles, same-ID/different-payload errors, trust takes weakest.

`session_summary` is fail-closed: its event tuple exists, but no immutable full
generated-surface binding does (`topic_segments` lacks `session_summary_id` and
quarantined segments may not persist). Every status/ack returns
`unverifiable_session_summary_provenance` before content/trust; no inference/
write occurs. Phase C needs a snapshot/FK; SELECT-only fixtures cover all states.

## Canonical Subject Mapping

Memory:

```text
(Memory,
 canonical owner_scope,
 canonical owner_key,
 Some(COALESCE(NULLIF(TRIM(scope), ''), 'project')),
 memory_type,
 CASE WHEN topic_key IS NULL OR topic_key = '' THEN memory:<id> ELSE topic_key END)
```

Only NULL/exact-empty keys are singletons; nonempty keys retain exact bytes, so
`foo`, ` foo ` and whitespace-only keys are distinct writer slots.

User claim:

```text
(UserContextClaim,
 exact owner_scope,
 exact owner_key,
 None,
 claim_type,
 claim_key)
```

A memory owner pair must be complete. Atomic legacy NULL/NULL fallback maps
global to `user/user:default` and other rows to `repo/memory.project`; a full
nonblank pair is authoritative. `owner_scope` is one of `user`, `workspace`,
`repo`, `tool`, `domain`, `workstream`, `session`; owner key is trim-stable and
nonblank, and executable DDL enforces all seven values plus ASCII-trim stability.
Normalized scope is exactly `project` or `global` (so `" global "`
normalizes to global). Unknown values and partial/blank pairs are contextual
integrity errors. Project `branch=Some(B)` admits neutral + exact-B rows;
`None` keeps all-branch behavior. Relation endpoints remain inside that set.

Scope dispatch and inclusion are exact:

| Query scope | Included rows | Required validation |
| --- | --- | --- |
| Project(P,B) memories | normalized memory scope is not `global`, and either `owner_scope='repo' AND (owner_key=P OR target_project=P)` or atomic legacy owner NULL/NULL with `memories.project=P`; then (`B IS NULL` or row branch is NULL/exact B) | `source_project` is provenance only; a routed full owner pair is authoritative, and `memories.project`, `owner_key` and `target_project` may legitimately differ |
| Project(P,B) observations | canonical `projects.project_path = P` through `observations.project_id`, with the same branch rule | when `project_id` is NULL, a nonblank legacy `observations.project = P` is the only fallback; when both identities exist they must agree |
| Owner(S,K) memories | canonical owner after the atomic legacy fallback equals `(S,K)` | all memory branches are included because Owner has no branch selector; no Observation rows |
| Owner(S,K) user claims | `owner_scope = S AND owner_key = K` | exact pair only; no Observation rows |

This is the v2 predicate, not byte-equivalence with legacy untrimmed context
SQL. A repo row can match by owner, target or both; differing nonblank
owner/target/placement is legal. The non-global guard applies to full and
legacy arms. Non-repo reroutes and global rows are Owner-only even if stale
placement/target names P. `source_project` is provenance only. The validation
probe includes placement/owner/target references to P and rejects partial
pairs. Blank full-pair target is absent; `target_project` never expands legacy
or non-repo membership.

For explicit `as_of=t`, Project/Owner membership and emitted `SubjectIdentity` use route-at-t from this logical schema (the migration uses the next available version):

```text
memory_route_ledger(
  id INTEGER PRIMARY KEY, memory_id FK ON DELETE RESTRICT, route_version,
  previous_route_id FK ON DELETE RESTRICT, effective_at_epoch, source_kind, audit_event_id scalar(no FK),
  source_writer_kind, source_ref, source_result_ordinal,
  source_fingerprint TEXT NOT NULL CHECK(length(source_fingerprint)=64 AND source_fingerprint NOT GLOB '*[^0-9a-f]*'),
  coverage_kind[complete|forward_only], coverage_start_epoch,
  placement_project, source_project, target_project, owner_scope, owner_key,
  memory_type, topic_key nullable, topic_domain, routing_confidence, routing_reason,
  context_class, memory_scope, branch,
  UNIQUE(memory_id,route_version), UNIQUE(previous_route_id),
  UNIQUE(memory_id,source_kind,source_fingerprint))
```
Each row is a complete route/identity state. Versions start at one, are contiguous, and link a same-memory predecessor. `source_kind` is the closed `insert|legacy_backfill|save_upsert|markdown_import|scope_cleanup` set; `source_ref` is the pre-write request ID (or deterministic migration identity), while audit IDs are diagnostics only. `memory_scope` is normalized per version; `topic_key` preserves raw NULL versus exact-empty bytes, and the cutoff version supplies both `memory_type` and topic key to `SubjectIdentity`. Required indexes are `(memory_id,effective_at_epoch,id)`, owner/target variants of `(owner_scope,<key>,memory_scope,branch,effective_at_epoch,memory_id)`, partial legacy `(placement_project,memory_scope,branch,effective_at_epoch,memory_id)`, and `(coverage_kind,coverage_start_epoch)`.
The FK-safe foreground migration snapshots dependent DDL/rows and verifies `foreign_keys=OFF` before one `BEGIN IMMEDIATE`; the Rust post-hook runs inside that transaction. It rebuilds `memories` without mutating dependents and runs `foreign_key_check` precommit; after commit the runner restores/verifies ON and repeats checks before writers. It may copy validated surviving creation/result and `scope_cleanup` evidence into creation and intermediate A→B→C states, but pre-cutover `save_memory` and Markdown updates had no exhaustive durable route log and `events` may already be pruned. Therefore `complete` is allowed only with exhaustive durable proof; otherwise only the migration-time current snapshot is `forward_only` at that floor. Missing evidence never invents history.
Before scoped discovery, an indexed probe returns `unreconstructable_routing_history` when any forward-only floor is after `t`, because that unknown prior route could match. Migration is marked applied only after every memory has one terminal row, complete chains validate, forward-only counts are reported, and terminal snapshots match `memories`.
After cutover, the canonical insert entrypoint opens the retry protocol before mutation and every inserted memory carries `insert_writer_kind TEXT NOT NULL`, `insert_request_id TEXT NOT NULL`, and `insert_result_ordinal INTEGER NOT NULL CHECK(insert_result_ordinal>=0)`, with `UNIQUE(insert_writer_kind,insert_request_id,insert_result_ordinal)`, a composite request FK `ON DELETE RESTRICT`, and an update-abort trigger making the tuple immutable (legacy backfill uses its deterministic migration identity). Literal `memory_insert_v1_ledgers AFTER INSERT ON memories` requires that tuple to match the open immutable request intent, calls the pre-registered deterministic `remem_sha256_frame_v1(name,value,...)` UDF over request hash, ordinal and exact typed NEW, then creates route and lifecycle version one inside the parent INSERT statement; either memory+both v1 rows exist or none do. Literal route/lifecycle fingerprint guards hash complete typed OLD/NEW rows except ID/digest. No fallback hash or post-insert ledger patch is legal. This covers the six current insert families in `memory/store/write.rs`, `memory/lifecycle.rs`, `memory_candidate/apply.rs`, `cli/actions/import.rs`, `markdown_archive.rs`, and `pack_import/active_import.rs`, plus future inserts.
All three production existing-row route writers use one reviewed canonical route-transition service: `memory::store::update_existing_memory`, Markdown `update_markdown_memory`, and `scope_cleanup::update_owner`. It NULL-safely compares actual OLD/NEW placement/project, branch, scope, source/target, owner, `memory_type`, raw `topic_key`, topic-domain/routing/context. A no-op needs no version; a real change appends the exact next snapshot and updates the row in one savepoint/transaction/epoch. Current normal-save target selection always requires the requested `memory_type`, so its reachable identity change is raw `topic_key` within one type; type change is accepted only on Markdown's validated stable-`source_id` path. A validated Markdown project→global change uses `source_kind=markdown_import`; scope cleanup also writes its audit mirror. Eval/test seeds insert their final route initially or use the fixture helper.
Literal `memory_route_tuple_update_guard` runs only when that tuple actually changes and requires one terminal staged next version from an unsealed request whose predecessor exactly matches OLD and snapshot exactly matches NEW; missing, sealed, wrong-head, OLD-mismatching, or NEW-mismatching rows abort. The seal guard independently requires every request-owned route/lifecycle row to be terminal and exact-match current memory tuple/status, so staging, row update, typed result and seal cannot cross transactions; null-safe no-ops pass. Any piece failing rolls back all pieces; ledger update/delete is forbidden.
Candidate discovery UNIONs the owner, target and legacy indexes before stable ID chunking; the per-ID chain is folded through `effective_at_epoch<=t` in `(epoch,id)` order, including scope for membership and SubjectIdentity. Invalid scope, a missing predecessor/source, gap/fork, nonmonotonic time, terminal drift, or legacy/forward-only coverage gap returns `unreconstructable_routing_history`, never current fallback; a complete scope transition does not.

## Lifecycle and Observation Mapping

Keep existing lifecycle mappings and add:

| Source/status | publication | validity | retention | visibility |
| --- | --- | --- | --- | --- |
| observations/poisoning_quarantined | Candidate | Unknown | Live | Suppressed |

Actual memory, user-claim and Observation adapters validate raw-status allowlists before projection. Unknown values return table/canonical-ref/raw-value context; a mapper’s generic Candidate/Unknown fallback is not adapter success.

Explicit memory history reads this independent durable projection:

```text
memory_lifecycle_ledger(
  id INTEGER PRIMARY KEY, memory_id FK ON DELETE RESTRICT, lifecycle_version,
  previous_lifecycle_id FK ON DELETE RESTRICT, effective_at_epoch, previous_status, new_status, source_kind, source_action,
  source_operation_id integer, source_api_operation_id text, audit_event_id scalar(no FK), source_writer_kind, source_ref, source_result_ordinal,
  source_fingerprint TEXT NOT NULL CHECK(length(source_fingerprint)=64 AND source_fingerprint NOT GLOB '*[^0-9a-f]*'),
  coverage_kind[complete|forward_only], coverage_start_epoch,
  UNIQUE(memory_id,lifecycle_version), UNIQUE(previous_lifecycle_id),
  UNIQUE(memory_id,source_kind,source_fingerprint))
```
Required indexes are `(memory_id,effective_at_epoch,id)`, `(coverage_kind,coverage_start_epoch,memory_id)`, and partial unique `(integer/API operation ID,memory_id)` bindings. The exact memory-status domain is `active|stale|superseded|archived|deleted|rejected`. `source_kind` is closed to `insert|legacy_backfill|memory_governance|web_governance|scope_cleanup`; memory governance requires its integer operation, Web requires the TEXT API operation and forbids the integer ID, scope cleanup has neither, and baselines additionally have no audit. The executable action matrix is closed: v1 `insert|legacy_backfill` uses `baseline`; memory governance allows `delete→deleted`, `reject→rejected`, `stale→stale`, or `acknowledge_pattern→same status`; Web allows only `active --archive→ archived` and `archived --restore→ active`; scope cleanup allows `archive→archived`, `reroute→same status`, or `memory_cleanup→active|stale` (the writer maps its existing `memory-cleanup` event label to this snake_case ledger value). Every nonbaseline row is v2+ with a nonnull previous status. Version one and a forward-only baseline have `previous_lifecycle_id=NULL`, `previous_status=NULL`, `new_status=<inserted/current status>`, and source kind `insert` or `legacy_backfill`.
General/Web governance and scope-cleanup archive/cleanup-plan writers append the next row with the status update in one transaction; `scope_cleanup::reroute_objects` appends a same-status row in its route transaction. Events are optional mirrors. Save/Markdown, candidate/TTL/supersede, preference removal, and stale-archive status writers remain legal but unsupported for exact history: terminal drift fails closed rather than a global guard breaking current behavior. Candidate replacement remains separately validated.
The chain starts at its validated baseline, joins `previous_status` to the prior `new_status`, ends at current status, and folds `(effective_at_epoch,id)` with equality applying the new status. A scoped memory with a forward-only floor after `t` returns `unreconstructable_memory_lifecycle`. Web rows copy the operation binding and exact-match durable `api_mutation_requests` resource/action/schema/response/status/time; its `audit_id` is correlation only. Unknown/unsupported/unrecorded transitions, gaps, forks, terminal drift, or Web mismatch return the same error.
Both ledgers have indefinite retention, are excluded from `cleanup_old_events`, and have no FK/cascade to `events`; event deletion or ID reuse cannot change proof. Memory/self FKs use `ON DELETE RESTRICT`, so cleanup cannot erase canonical history. A future purge requires a separately reviewed tombstone/compaction migration, never cascade. Regression calls `cleanup_old_events_at` past 30 days and proves identical route/lifecycle output, intact Web proof, zero ledger deletes, and `foreign_key_check`.

Both ledgers use one retry protocol. `source_fingerprint` is lowercase SHA-256 over an ordered binary frame: field-name length+bytes, type tag and value length+bytes; integers are signed big-endian, reals IEEE-754 big-endian, strings are exact raw UTF-8 bytes, and NULL differs from empty. `remem_sha256_frame_v1` accepts nonempty unique field-name/value pairs, encodes SQLite NULL/INTEGER/REAL/TEXT/BLOB with that frame and returns exactly 64 lowercase hex. It is registered with deterministic/innocuous flags immediately after keying every owned connection and before migration; an unregistered raw writer fails closed. The frame covers schema/ledger version, memory/source kind+action, predecessor ID/version, stable request ID, strict request fingerprint, result ordinal, and complete typed OLD/NEW transition state. It excludes the request-wide result fingerprint, response and later generated IDs, which enter the final seal. There is no generic CRLF/trim rule; only production-canonicalized fields use documented canonical bytes. Ordered arrays retain order/duplicates; declared sets alone sort/deduplicate bytewise.

| Writer | Canonical request discriminator |
| --- | --- |
| insert / legacy backfill | pre-write request/operation ID + complete canonical insert request / migration version + memory ID + complete baseline |
| save / Markdown | required caller `idempotency_key` derives opaque namespaced request ID but is excluded with credentials from payload hash; hash every other exact raw `SaveMemoryRequest` value `text,title,project,session_id,host,topic_key,memory_type,files,scope,created_at_epoch,branch,local_path,local_copy_enabled,claim_enabled,claim_source,acknowledge_pattern` plus separately supplied raw `reference_time_epoch`, Option presence, file order/duplicates, raw CR/LF/outer whitespace and effective adapter/default inputs; result hash covers exact final `SaveMemoryResult`/serialized response, memory/operation/route/lifecycle/claim/ack/local-copy/next-step rows and digests / Markdown uses stable source binding (`source_id`+creation/reference; prior source hash only a lookup precondition) or no-source identity (export version + canonical archive-relative path + synthesized topic) plus its post-render semantic frame; importer-owned metadata is excluded while byte-preserved fields remain exact |
| general / Web governance | action + actor + normalized reason + acknowledgment pattern + sorted target set / durable operation idempotency identity + canonical request hash |
| scope reroute / archive / cleanup plan | action + object ref + normalized owner/target/topic/routing/context/reason / action + object ref + normalized reason / planner version + canonical plan/group snapshot hash |
The exact executable tables, immutable lock-anchor identity, manifest validator, typed binding shape, append-only triggers and seal guards are normative in `MIGRATION-CUTOVER.md`. Intent stores an expected `(result_ordinal,binding_kind)` manifest; typed results bind insert origins, route/lifecycle transitions, integer/API operations, audit provenance, memory/claim/ack/local-copy outcomes and response auxiliaries. Ledger INSERT guards require an open request and compatible manifest slot; the seal guard requires exact set equality, verifies every route/lifecycle row has its manifest-declared typed result (including insert v1 pairs and exact operation/audit fields), is terminal and exact-matches current memory route/status, and rejects missing, extra or shape-invalid bindings. UPDATE/DELETE abort on anchor/intent/result/seal/ledgers; dedicated BEFORE INSERT conflict guards also reject every `INSERT OR REPLACE` unique collision with `recursive_triggers=OFF`, and no ledger can append after seal.
Caller-facing save accepts a required 1–128-byte ASCII idempotency key using `[A-Za-z0-9._~-]`, trims it once, derives `save_<sha256(namespace||key)>`, and never persists/logs/returns the raw key. The key and credentials are excluded from `request_fingerprint`; every behavior-bearing payload byte and effective adapter input is included. Same request ID/hash replays the stored canonical response; same ID/different hash conflicts before mutation; different IDs with identical payload execute independently. The cross-writer request ledger may accept a wider identifier alphabet, but every derived/internal local-copy request `R` and immutable K key separately match `[A-Za-z0-9][A-Za-z0-9_-]{0,127}` exactly; internal writers use reviewed stable source/plan identities, never generated result IDs.
On a miss, `BEGIN IMMEDIATE` appends intent before mutation; INSERT triggers create v1 immediately, then writers append every manifested binding and final seal. `save.rs` owns database, claim, ack and response construction. Every direct save, regardless of local-copy option, first acquires a candidate retained OS lock; a short anchor transaction must match its fd/path `(dev,ino)` and durable nonce to immutable `memory_write_lock_anchors`. Only a virgin-R direct save may initialize absent K: a no-transaction preflight and serialized recheck inspect exact request/result/commit and `J/T/Tc/V/Xc/U/G/O/S/B/N/C/H` existence, while the same transaction rejects candidate IL owned by another R before nonce mutation; scanner/doctor require K and unlocked inspection is forbidden. Secure first creation and parent fsync make Q, Q/locks and Q/quarantine mode-0700 durable. The writer holds L through cleanup/reconciliation. A journal-durable `stage_building` creates nonce-qualified U; fdatasynced D1 publishes no-replace as request-qualified S, durable `new_pin_intent`/`new_pinned` pins N, portable absent publication accepts `{target,S,N}` nlink=3, and present publication uses proved request-qualified B plus atomic exchange. Recovery never reverses exchange: N pins D1, restore pins C, evacuation uses H, EEXIST preserves a newer target, no-seal rollback permanently retains G, and matching-seal prior-file cleanup permanently retains O before removing the predecessor pins. Before any final-pin unlink, canonical source J records request/format fingerprints, epoch, source phase, `before_kind`/publication/expected-seal state, `semantic_d0_digest`/`semantic_d1_digest`, exact trusted-root/component/path proofs, and `source_namespace` entries containing only dev/ino/uid/gid/type/nlink; structural alias groups and nlink are checked before J creation or chmod, while current mode/size/mtime/digest are first frozen in the cleanup snapshot. Cleanup conversion admits only the five source/list tuples `[B]`, `[H,S,B,C]`, `[S,N]`, `[N]`, and `[H]`. It snapshots one ordered cleanup contract and revalidates every fsynced prefix. Cleanup alone uses nonce-qualified `Tc=Q/.R.<stage_nonce>.cleanup.tmp`: a request-wide scanner reserves every same-R cleanup-temp candidate across nonce/name forms; under held L, canonical source J and exact private-Q/type/uid/gid/device/mode/nlink ownership, only the sole exact current-nonce candidate may have provisional bytes discarded and Q-fsynced, only its fresh complete exact document replaces J, and stale/malformed/multiple candidates or any same-R Tc beside canonical cleanup J are preserved ambiguous. Any unreadable source or cleanup hash first exact-matches its pathname and retained write FD to the expected full raw snapshot proof, then no-replace hard-links canonical `J→V=Q/.R.<stage_nonce>.read-lift.<group>.<mode:04o>`, fsyncs Q, rechecks the bound alias/mode, and adds only owner-read; V may coexist with Tc. A lexical request-wide scanner reserves every same-R V across nonce/name forms, including extra-dot malformed prefixes while isolating distinct valid request prefixes. Restart accepts only the unique exact current-nonce V, restores and fsyncs its encoded mode through a surviving exact structural alias before removing V, retains V on restore fchmod/fsync failure, and only then reports drift; stale/malformed/multiple candidates are preserved ambiguous. Without V, the same read-bit change is drift. Detectable snapshot drift returns typed `local_copy_cleanup_concurrency_violation`; journal/path/identity ambiguity returns typed `local_copy_reconciliation_ambiguous`; both retain J/pins and expose `doctor_healthy=false`. Permanent G/O mutable fields remain exempt because their names survive. J has one closed phase and no goal; the DB seal remains authoritative, all unlisted phase×seal/path/alias tuples fail closed, active same-uid nonprotocol mutation inside private Q remains outside the threat model, and sealed exact replay is mutation-free with a fresh stage nonce for a fresh attempt.
The local-copy cleanup call surface has no unlocked/default variant: inspection, source-J prepare/load/transition, read-lift begin/finish/recovery, snapshot proof, cleanup-intent persistence/revalidation, capture/restart, ordered cleanup and J unlink require a `HeldRequestLock` capability bound to R and its acquiring PID. Because POSIX record locks are process-scoped, acquisition atomically reserves `(Q/locks dev,Q/locks ino,R)` in a process registry before opening L; a same-process duplicate opens/closes no L FD, and this reservation never substitutes for the kernel lock. The child at-fork callback closes and invalidates every inherited capability FD before resetting the registry, so closing an old child object cannot close a recycled FD or release a fresh child lock. The capability retains a dup of the exact mode-0700 `Q/locks` FD, proves its reservation and L as the same current-uid 0600 regular nlink-1 fd/path inode with a live exclusive kernel lock, and at every entry, callback, rename/unlink and journal boundary no-follow-opens canonical absolute Q, matches its inode to the retained Q handle, and matches canonical `Q/locks` to the retained locks-directory inode. Closed, fork-inherited, wrong-request, replaced-L, decoy-locks or replaced-Q capability returns typed lock-unsafe before mutation; L is never unlinked and SIGKILL releases only the kernel lock. Capture/restart independently require exact canonical cleanup J, its complete field set, trusted-root/directory-handle proofs and logical path bindings, its allowlisted contract, no forbidden V/Tc/Xc coexistence, and the exact next ordered member with every prior absent and every later present. Each ordered cleanup entry uses `Xc=Q/.R.<stage_nonce>.cleanup-capture.<H|S|B|C|N>`: retain the exact read/hash proof (using V for 0200), native atomic no-replace rename source→Xc, fsync Q then source parent, prove source absent and Xc plus retained FD still exact, then unlink Xc and fsync Q. Unsupported native no-replace support fails closed. Mismatch restores Xc→source no-replace and fsyncs source parent then Q; EEXIST retains Xc and returns concurrency. Restart restores the sole valid current-uid regular same-device Xc before removed-prefix derivation; malformed/multiple/stale-nonce/symlink/unsafe/out-of-order Xc is preserved ambiguous. The Xc proof deliberately accepts nlink≥1 without binding its inode to the snapshot so a replacement captured in the proof→rename window is restored before reporting drift. Runtime expected nlink is recomputed from all still-named snapshot aliases, including permanent G/O, after every prefix. Snapshot construction and every cleanup revalidation run two complete ordered passes over all entries and all present/absent predicates: the first may invoke the designated callback, the second is callback-free, and success requires both passes and the expected state to agree. Public scalar inputs are exact built-in `str`/`int` values, never `bool` or subclasses, and request, nonce, basename, group and mode must satisfy their canonical grammar/range before proof. `load_cleanup_journal(path_contract=None)` remains the sole compatibility bootstrap; each Q-only lookup re-proves held L plus canonical Q and `Q/locks` immediately before and after access, while any supplied contract is fully validated. V+Tc is valid; V+Xc and Tc+Xc are ambiguous. Every production safety invariant is an explicit typed check rather than an AST assertion or hand-raised AssertionError; public journal/snapshot/inspection/transition/read-lift/cleanup boundaries map external proof failures to typed errors and remain active under optimized execution. Same-uid nonprotocol mutation inside private Q, including check-to-Xc-unlink substitution, is outside the threat model; replacement of canonical Q observed at any mandated boundary still aborts without touching either old or replacement J.
Every owned descriptor close moves and invalidates its owner before exactly one close attempt; no returned numeric FD is probed or retried, all distinct siblings receive one attempt, and capability/registry state is released after ownership consumption. Among close-only failures the first is preserved and later failures remain diagnostic. A body/callback error keeps its original identity and outranks close failures; a restoration/finish safety error instead outranks the callback while retaining that callback diagnostically. At a public revalidation boundary, callback failures become typed `local_copy_reconciliation_ambiguous` with the original exception as cause, never `False` or cleanup-concurrency. In snapshot construction or cleanup revalidation, before restoring V, a fresh descriptor proof must still show the exact lifted mode; a third mode is preserved as drift with V armed and is never overwritten. Independently, every operation naming P reopens canonical trusted-root→P and exact-matches retained P before path-dependent mutation, recovery, and successful return, including inspection and V begin/finish/restore/recovery; retained P alone is not authorization.
For the local-copy contract above, virgin-R preflight and its serialized recheck enumerate every Xc-prefix candidate in addition to J/T/Tc/V/U/G/O/S/B/N/C/H. Post-exchange `B`/`S` need only prove the same structural predecessor `I0*`: phase-qualified mutable old-FD drift never blocks sealing, and permanent `O` preserves that state. Ordinary phase updates retain distinct `T=Q/.R.json.tmp` semantics; cleanup never overloads T. After request-wide scans, source J may restart with Tc absent or one exact current-nonce Tc containing arbitrary provisional bytes, and the sole exact current-nonce mode-qualified V may coexist so its encoded original mode is durably restored before Tc classification; stale/malformed/multiple V or Tc candidates are preserved ambiguous. Canonical cleanup J requires every same-R Tc absent and admits only absent V or one exact current-nonce group/mode-qualified same-inode J/V nlink=2 marker. Cleanup J plus Xc restores Xc first; source J plus Xc, Tc+Xc, V+Xc, and any Xc at source-J transition or J unlink are ambiguous. Immediately before J unlink, revalidate full canonical intent/path binding and request-wide absence of every Tc/V/Xc candidate; a late stale Tc preserves itself and J. Target replacement/open-FD drift restores V and any captured source through a surviving/no-replace alias before the typed error is returned.
A scoped Observation with `created_at_epoch=NULL` is a contextual integrity error because public knowledge time is non-null; human-readable `created_at` is not a fallback. Separately, `reference_time_epoch=NULL` validly falls back to required creation.

Active Observation mapping:

```text
evidence_ref             = observation:<id>
kind                     = Observation
source_ref               = observation:<id>
scope                    = Project { canonical project, branch }
lifecycle                = Some(observation_lifecycle(status))
source_time_epoch        = COALESCE(reference_time_epoch, created_at_epoch)
knowledge_time_epoch     = created_at_epoch
trust                    = ModelGenerated with no refs; otherwise min(ModelGenerated, weakest supporting-event trust)
integrity                = Validated only after a current read-time scan
supporting_evidence_refs = validated captured_event refs, sorted/deduped
```

Stale/Archived rows do not enter a current-snapshot catalog. Explicit history
requires complete transition history for any scoped stale/compressed row created
by the cutoff, else `unreconstructable_observation_lifecycle`. Compression
snapshots omit prior status and stale transitions lack timestamps, so neither
proves it. Quarantined content stays filtered before catalog/attachment/trust.

Observation `evidence_event_ids=NULL` means no refs; otherwise require a JSON
positive-integer array, then sort/dedup/validate. Each event's source/insertion
is no later than Observation binding/query, and canonical project matches. If
any Observation host/project/session ID exists, require all three, a matching
session and exact event triples; all-null is the legacy path. Partial/dangling/
cross-host identity errors. Canonical reclassification makes external support
Untrusted, never an uplift.

Historical active rows lack a poison scan version, so re-scan all generated
surfaces before Validated output. A hit returns contextual poisoning error.
Stored `poisoning_quarantined` remains filtered Quarantined policy state.

Observation is never a `ClaimSource`. The only attachment is a same-project
`memory_facts` row with both
`source_memory_id=<memory id>` and
`source_observation_id=<observation id>`. Both endpoints must already belong to
the scoped/time-eligible input set and `fact.project` must equal the query,
memory placement and canonical Observation project. Direction is Observation
supports Memory Claim. Shared events, text/topic similarity and model inference
do not create links. Phase A has no Observation→UserClaim attachment.

At reference `t`, link eligibility reuses the semantics of
`memory::facts::as_of_validity_filter_sql`:

```text
learned_at_epoch <= t
AND created_at_epoch <= t
AND (valid_from_epoch IS NULL OR valid_from_epoch <= t)
AND (invalidated_at_epoch IS NULL OR invalidated_at_epoch > t)
AND (
  valid_to_epoch IS NULL OR valid_to_epoch > t
  OR (
    invalidated_at_epoch > t
    AND no replacement with supersedes_fact_id=<fact id>
        and learned_at_epoch <= t
  )
)
```

Validate fact status/predicate, replacement and endpoints. The real schema's
NOT-NULL `created_at_epoch` is actual insertion; learned/updated/human time
cannot substitute. Missing-column/NULL legacy data fails with table/fact/field
context. A replacement must also be learned and inserted by `t`. Tests cover
all boundaries and a late-inserted backdated fact.
## Evidence Trust

Captured-event classification reuses the canonical
`SourceTrustClass` logic from `src/memory/poisoning.rs`:

| Source class | Evidence cap |
| --- | --- |
| user_prompt | Verified |
| repo_file | Verified |
| local_tool_output | Verified |
| pack | Untrusted |
| external_content | Untrusted |

WebFetch/WebSearch, `mcp__*`, network-fetching Bash and session-stop are
external. Truth reconstructs full content before classification. `raw_keep` is
exactly `full_content_byte_length<=16384`, no blob, full `content_text` and its canonical
SHA-256 event hash. `raw_compact` is `full_content_byte_length>16384`, with a plain UTF-8 blob, both byte
counts equal to its length, canonical preview/event SHA-256, and matching
SHA-256 or exact 16-hex legacy blob hash. Dangling/crossed retention/blob state,
encoding, length, preview or hash errors. Only the capture constant/pure preview
helper and poisoning pure classifier may become visible; writer behavior stays
unchanged. Tests lock 16384/16385, multibyte boundaries, and a network marker in
the compacted middle absent from both stored preview ends.
For each memory, parse the stored class and independently reclassify every
referenced captured event with the current canonical classifier:

```text
stored_cap = cap(parse(memories.source_trust_class))
recomputed_event_cap =
  min(cap(canonical_class(event)) for every referenced event), if any
effective_source_cap =
  min(stored_cap, recomputed_event_cap), or stored_cap when no event exists
strongest_evidence =
  max(eligible validated non-SourceTrustClass evidence trust,
      default ModelGenerated)
effective_claim_trust =
  min(strongest_evidence, effective_source_cap)
```

Candidate-backed memory additionally includes the validated candidate’s stored
source cap in `effective_source_cap`; later trust rewrites cannot uplift it.
The weaker recomputed cap protects rows whose v060 migration/default stored
`local_tool_output` even though their actual refs include WebFetch/MCP/network
Bash. `SourceTrustClass` Evidence is diagnostic only and is excluded from
`strongest_evidence`, so it cannot self-uplift. The cap never uplifts an
evidence-less claim. External/pack remains Untrusted in mixed provenance; stored confidence never participates.

Captured events join canonical project. Expected project is nonblank `memory.source_project`;
only atomic unrouted legacy may use `memory.project`. Missing/foreign/ambiguous/
routed-without-source refs and malformed/dangling IDs error. A present
`source_candidate_id` must resolve a completed `auto_promoted|approved|edited`
candidate with exact evidence, content/confidence and accepted input route.
The route-ledger initial state—not today's row—must exact-match candidate/result
owner, project, scope, type and raw topic key; its scope equals
`CandidateRoute::memory_scope()`: global only for user owner, otherwise project.
Candidate scope and its unpersisted derived title are not copied-equality fields.
An exact `memory_operation_log(source='memory_candidate',
source_candidate_id=candidate.id,result_memory_id=memory.id)` binds that initial
state; workspace/pack fixtures lock scope/title rules. At query cutoff, fold the
complete chain and validate identity/membership against that version, allowing
proved owner/project/scope/type/raw-key transitions. Current reads also require
terminal full-tuple equality with `memories`; chain/coverage/terminal failures
use `unreconstructable_routing_history`, while unexplained content/provenance
drift is `unverifiable_post_candidate_mutation`. Refs bind at candidate creation;
completion/cleanup knowledge is separately reference-eligible. Without a candidate,
a compatible durable result operation binds refs. With neither, current
`as_of=None` binds refs at `reference_epoch`; history excludes/Unknown. Malformed
claimed links error; event source/insertion must not exceed binding/reference.
## Temporal Rules

- Define `effective_memory_knowledge_epoch` once for every memory ClaimView,
  SourceRef check and SourceTrustClass view. Proof is a validated candidate
  completion plus route chain, or a `memory-operation-planner-v1`
  `add|update|conflict` row exact-matching result ID and route/identity state
  effective at the operation epoch; a later legal route version does not erase
  that proof. Mismatches and other planners are non-proofs. A canonical `noop`
  requires that planner, result ID/identity at its own epoch, empty transitions,
  `noop_reason='already represented by active memory'`, and source tuple
  `direct/save_memory/NULL` or exact `memory_candidate/memory_candidate`
  plus matching noop candidate; input/result topics may differ. It proves only
  the transition.
  Knowledge is the max of earliest ingestion proof, eligible noops, memory
  update, candidate completion/ack, route-ledger versions and complete/current memory
  ack; partial/stale ack errors. No proof means historical exclusion/Unknown
  and current `reference_epoch`. Memory source remains
  `COALESCE(reference_time_epoch,created_at_epoch)` and cannot be future.
  Direct-save noop, governance ack and candidate ack use their canonical
  operation, memory update and candidate update epochs respectively.
- UserContextClaim source is `COALESCE(valid_from_epoch,created_at_epoch)`.
  Edited descendants retain provenance-root SourceRefs; transitions change
  ClaimView state knowledge, never inherited-ref binding.
- Before a versioned transition, the predecessor has pre-transition lifecycle
  and creation knowledge. At equality the successor is current; a retained
  rejected predecessor has transition knowledge and Superseded lifecycle.
  Mutated predecessor update time is an edit boundary, not SourceRef knowledge.
- A non-governance in-place claim suppress/unsuppress/delete after the cutoff
  cannot reconstruct prior state and is excluded/Unknown; at/after its update,
  current ClaimView knowledge may use that update while SourceRefs keep their
  provenance-root binding. Audited memory governance instead uses the complete
  lifecycle chain above.
- Captured-event source is `COALESCE(reference_time_epoch,created_at_epoch)` and
  knowledge is original insertion. Replay of `(host_id,session_id,event_id)`
  preserves timestamps/payload but may append keyed Git evidence/work. Pre-v2
  insertion is a conservative floor; both clocks must be reference-eligible.
- A one-to-one `edit_claim` chain selects old before transition and successor
  at/after it. Missing/forked/cross-owner/timestamp-inconsistent chains error.
- Candidate application is a separate canonical multi-row transition. An
  applied candidate (`auto_promoted`, `approved` or `edited`) must have an exact
  owner/type/key result row, `result_claim_id`, and one shared transition epoch
  equal to the candidate update and every changed predecessor update.
  - Replacement: the result is a candidate-derived claim created at the
    transition. Active rows are ordered `(updated_at_epoch DESC,id DESC)`; the
    first is the required `supersedes_claim_id`, while every same-identity row
    Superseded at that epoch is a co-predecessor. Before transition all eligible
    co-predecessors are active; at equality the result replaces them.
  - No-op: `result_claim_id` names a pre-existing exact-text/sensitivity active
    claim: the first exact match in that same order. It stays unchanged while
    other active rows are Superseded; before transition they are active, at
    equality the kept result remains current. Candidate provenance does not
    replace the kept claim’s SourceRefs.
  - An unlinked Superseded row is accepted only when this authoritative
    candidate/result/timestamp pattern validates. Otherwise historical state is
    not reconstructable and the projection returns a contextual integrity
    error instead of silently dropping or inventing a predecessor.
- General hard deletion/content rewrite without durable history remains
  unreconstructable; Phase A returns less rather than guessing.

All claim, relation and fact event-validity windows are half-open:
`valid_from_epoch <= t` and `valid_to_epoch > t`; equality at `valid_to` is
expired/not effective. Source/knowledge equality is eligible, and a user-edit
successor becomes effective at transition equality. These boundaries are
tested separately from suppression revocation equality.
## Policy Suppression

The seven canonical target kinds and stored shapes are exact:

| target_kind | canonical shape | match |
| --- | --- | --- |
| `memory` | positive `target_id`; `target_value=NULL` | exact `memories.id` |
| `topic_key` | `target_id=NULL`; trim-stable nonblank value | byte-exact `memories.topic_key` |
| `entity` | `target_id=NULL`; trim-stable nonblank value | SQLite `lower()` equality with a linked `entities.canonical_name` |
| `pattern` | `target_id=NULL`; trim-stable nonblank value | SQLite `instr(lower(field),lower(value))>0` over memory title/content or user-claim text/key |
| `user_claim` | positive `target_id`; `target_value=NULL` | exact `user_context_claims.id` |
| `user_candidate` | positive `target_id`; `target_value=NULL` | exact `user_context_candidates.id` |
| `summary` | exactly one of positive `target_id` or trim-stable nonblank value | `user_context_summaries.id`, or value equal to its decimal ID / case-insensitive substring of `summary_text` |

Extra/both/missing ID/value combinations are errors. Memory/topic/entity/
pattern and user-claim targets change a Claim’s Visibility without changing
validity. `user_candidate` and `summary` are validated against the tables above
but non-applicable in Phase A because neither row has Claim standing; they do
not transitively suppress a candidate-derived claim, a `session_summaries`
SourceRef, or any evidence. Unknown kinds are errors.

- `(owner_scope, owner_key)=(NULL,NULL)`: global;
- both non-null/nonblank: exact owner only;
- partial pair: contextual error.

For an exact owner, a direct-ID target row must have that canonical owner and
value matching is evaluated only inside that owner. For global `(NULL,NULL)`,
an exact direct ID may match its target row under any owner, and value matching
has no owner restriction; query scope still bounds the claims that can be
hidden. A missing direct target, owner mismatch, unknown status or invalid
shape is an error. Tests cover every match rule, global/exact-owner direct-ID
semantics and both recognized non-applicable kinds.

Historical non-entity target match at `t`:

```text
created_at_epoch <= t
AND (
  status = 'active'
  OR (status = 'revoked' AND t < updated_at_epoch)
)
```

At revocation equality the target is visible again.
`memory_entities` has no link time. Any applicable current entity target makes
the projection `CurrentSnapshotOnly`; an effective historical entity target
returns `unreconstructable_entity_link_history` unless durable history proves
scoped membership/non-membership. Current links/entity creation/backfill are not proof.
## Relations and Resolution

The `memory_edges` writer domain is closed and mapped exactly:

| raw `edge_type` | DTO kind | DTO direction |
| --- | --- | --- |
| `supersedes` | Supersedes | stored old→new becomes new→old |
| `duplicates` | Supports | stored from→to |
| `conflicts` | Refutes | stored from→to |
| `derived_from`, `merged_into`, `split_from` | DerivedFrom | stored from→to becomes to→from |

The bounded query first finds every row touching a scoped memory ID and parses
its raw kind before endpoint/output filtering. Unknown, newer or typo kinds
(including graph-only `extracted_from`) return context containing
`table=memory_edges`, edge ID and raw value. A known candidate-provenance
`derived_from` row may have a NULL source endpoint; validate it but do not
invent a Claim endpoint. Only parsed relations with both endpoints in the
scoped claim set are emitted.

- Supersedes and ordinary Refutes decide only inside one exact identity.
- Scoped Supports/DerivedFrom may connect different identities and are attached
  as provenance when they touch a winner; they never change survivor/trust/
  recency.
- A canonical operation-backed cross-topic preference conflict is the sole
  cross-identity decision exception. Both endpoints must share owner, normalized
  scope and normalized branch (`COALESCE(branch,'')` byte equality), have type
  preference, and survive their own slots. The post-pass marks both outputs
  Contradicted without merging slots.
- A conflict with `source_operation_id` validates operation kind, integer ID
  array, linkage and replacement/pairwise membership. Claimed-but-missing,
  malformed or inconsistent provenance is an error. An unbacked conflict edge is
  decision-neutral.
- Canonical heterogeneous graph/dream pairwise conflicts may legitimately use
  fallback operation owner/type metadata. Valid linkage/membership is accepted,
  but the relation stays decision-neutral because the uniform preference
  predicate is false.
- The approved uniform-conflict graph must be a matching after parallel edges
  for the same endpoint pair are collapsed. Any survivor with two distinct
  partners (for example A-B and A-C) is ambiguous canonical state and returns a
  contextual integrity error.

Approved cross-topic outputs keep their own subject, have `claim=None`,
Contradicted validity, sorted/deduped survivor evidence union, no supporting
relations, the validated connecting conflict set, prior per-slot rejected refs,
both survivor refs and `UnresolvedConflict`; paired outputs share relation set.

Unbacked means both source IDs NULL, uses edge creation knowledge and is
decision-neutral; candidate-only errors. A source operation must exist, be
eligible and precede/equal edge creation; operation-only is valid. A claimed
candidate matches its operation discriminator/ID and proves canonical memory
status/result/operation/endpoints or graph status/promoted-edge/operation, and
was created no later than the operation. Knowledge is max(edge/operation
creation, validated application update), is eligible, and may follow edge
creation. Boundary fixtures reject dangling/future/mismatch bypasses.

Resolver order:

```text
scope/time/lifecycle eligibility
  -> exact-identity supersedes
  -> exact-identity refutes
  -> effective evidence trust
  -> recency
  -> cross-topic preference conflict post-pass
```
## Bounded SQL Contract

Claims, route coverage/discovery/history, raw edge validation, captured
evidence, facts, suppressions and lifecycle-ledger rows use scoped IDs/owners/
projects and the named indexes, never unrelated scans plus `Vec::contains`.
One SQLite snapshot covers the first scoped-claim SELECT through final resolve.
Autocommit owns deferred BEGIN and terminal COMMIT/ROLLBACK; an existing caller
transaction is reused without nested control or committing caller work.
ID chunks are stable ascending chunks of at most 900. Seed-933 contains 901
target memories, 1,802 relations, 901 evidence refs and one 900-link high-fanout
subject, plus 4,505 unrelated memories, 9,010 relations and 4,505 evidence refs.
Required structural checks:
- authorizer permits read/SELECT and only the owned transaction controls;
  `total_changes` remains unchanged and every DML/DDL attempt is denied;
- each data plan is an indexed scoped/ID search, each bind count is at most 900,
  and no unrelated row is returned/materialized; route/lifecycle plans must
  report index SEARCH and must not SCAN `events`;
- data-statement count is at most `12 + 5*ceil(scoped_claims/900) +
  2*ceil(scoped_evidence_refs/900)`;
- transaction-control count is exactly two for autocommit (BEGIN plus terminal
  COMMIT/ROLLBACK), zero inside a caller transaction, and reported separately;
- adding unrelated rows changes neither counts nor serialized target output.

Final-head record:

```bash
GH933_PERF_JSON_OUT=/tmp/gh933-truth-perf-v2.json \
  cargo test truth_performance_contract --release -- --ignored --nocapture
```

Use five warmups and 50 measured runs. JSON schema v1 includes exact head SHA, seed/counts, chunk size, Rust/SQLite versions, plans, statement/bind/row counts, bytes, p50/p95, fingerprints and structural booleans.
The Rust test validates it; latency is informational, and any later commit/base sync invalidates the record.
## Versioning and Golden Diff

v2 ships in 0.7.0 or the next explicit breaking boundary. Sync Cargo, lockfile, plugin, runtime manifest, npm wrapper, `server.json`, changelog, README and architecture docs. The guide covers ClaimView `created_at_epoch`/`updated_at_epoch` becoming version-specific source/knowledge epochs, edit transitions, in-place limits and recency.

`tests/truth_public_api.rs` acts as an external crate and compiles `use remem::truth::{...}` for Project and Owner queries.

Allowed v1→v2 golden differences:

1. version, exports, TruthScope, typed subject/exact selector and effective epoch/replayability;
2. EvidenceView/ClaimView temporal fields and Observation catalog/read-scan/nullable-epoch/trust/attachment;
3. identity isolation, canonical Project inclusion, indexed historical route/backfill (intermediate states and forward-only failure), Owner union, global/legacy fallback and wrapper;
4. edit/candidate reconstruction, globally ordered general/Web/scope-cleanup lifecycle recovery and conservative unsupported mutation handling;
5. suppression owner/time visibility;
6. source-trust cap, binding-time checks, first-party rules, candidate/edit invariants, summary failure and full-blob fix;
7. fact attachment's actual `created_at_epoch` eligibility gate;
8. total six-kind edge mapping/direction, heterogeneous conflict error→neutral and cross-topic/overlap output;
9. approved malformed/dangling/unknown fail-closed/error output.

Everything else, including branch semantics and resolver order, must remain
field-by-field identical. Do not replace the whole golden to hide drift.

## Verification

```bash
cargo fmt --check
cargo check
cargo test truth -- --nocapture
cargo test --test truth_public_api
cargo test context
cargo test
cargo clippy --all-targets -- -D warnings
python3 scripts/ci/check_plugin_version_sync.py
python3 scripts/ci/check_version_bump.py origin/main HEAD
python3 scripts/ci/check_pr_preflight.py --base origin/main \
  --pr-body-file /tmp/pr-body.md
```

Focused regressions cover the writers, routing/lifecycle, retry, DDL, UDF, local-copy, trust, fact, and edge cases above. `MIGRATION-REHEARSAL.md` supplies
exact-head negative/crash evidence; `ROLLOUT.md` requires independent review,
fresh CI, and separate human merge/release/cutover authorization.

## Phase Boundaries and Rollback

Phase B defines Context Bundle, shared render epoch, selectors, visible failure, budget/cache/history and rollback. Phase C decides general Claim-writer convergence beyond Phase A's narrow history substrate.
Projection stays SELECT-only; Phase A adds both ledgers, backfill and guards; `ROLLOUT.md` disables v2 reads without dropping history and forbids 0.6.x after a seal.
