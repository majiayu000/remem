# GH933 Technical Contract — CurrentTruth Projection

Refs #933. `PRODUCT.md` defines product behavior. This file is the normative
Phase A v2 implementation contract.

## Status and Module Boundary

Truth v1 shipped publicly in `remem-ai` 0.6.26/0.6.27. The Cargo library target
is `remem`, so the public API is `remem::truth`.

The Phase A v2 projection remains a read-only adapter over existing tables:

```text
src/db/capture.rs (duplicate-event timestamp immutability + pure preview helper)
src/truth.rs
src/truth/adapter.rs
src/truth/lifecycle.rs
src/truth/projection.rs
src/truth/types.rs
src/truth/tests/**
tests/truth_public_api.rs
```
No projection query may write/migrate/call external systems/change Context
Bundle; duplicate captures preserve row timestamps, and no other writer changes.
Split `src/truth/tests.rs` before v2 tests; every source file stays below 800 lines.

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
nonblank. Normalized scope is exactly `project` or `global` (so `" global "`
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

For explicit `as_of=t`, Project/Owner membership and emitted `SubjectIdentity` use route-at-t. Candidate discovery is bounded by canonical candidate/result
creation proof plus current route; `scope_cleanup` reads are stable-ID/source-project
bounded. Events require `event_type='scope_cleanup'`, `object_ref='memory:<id>'`
and complete previous/new snapshots of the writer's eight fields:
source/target project, owner scope/key, topic domain, routing confidence/reason
and context class. Order by `(created_at_epoch,events.id)`; first previous
equals creation route, adjacent new/previous snapshots match, and terminal new
equals current row. Fold epochs `<=t`, so equality uses new. Since the writer
neither snapshots nor mutates `memories.scope`, normalized creation-proof scope
is invariant. Missing/forked/contradictory/dangling/scope-changing history is
`unreconstructable_routing_history`; never fall back to current route.

## Lifecycle and Observation Mapping

Keep existing lifecycle mappings and add:

| Source/status | publication | validity | retention | visibility |
| --- | --- | --- | --- | --- |
| observations/poisoning_quarantined | Candidate | Unknown | Live | Suppressed |

Actual memory, user-claim and Observation adapters validate their raw status
allowlists before projection. Unknown values return table/canonical-ref/raw
value context. A mapper’s generic Candidate/Unknown fallback is not a silent
adapter success.

Explicit memory history loads ID/project-bounded `memory_governance` events,
strict-parses memory/action/previous/new status and orders by
`(created_at_epoch,events.id)`. The chain starts at validated creation/result
status, joins every previous to prior new and ends at current status.
`delete/reject/stale` map to `deleted/rejected/stale`; acknowledgment is a
validated same-status event; Web archive/restore are active→archived and
archived→active. Every Web event binds its operation and event ID to exactly
one `api_mutation_requests` row matching operation, `resource_kind=memory`,
resource ID, action, schema-1 and audit ID. Response operation/audit/memory/
action/before/after/version/occurred-at plus `replayed=false`, ledger/event time,
project and `api:<operation_id>` session all match. Validate
the terminal archive marker (or its clearing after restore). Fold epochs
`<=t`; missing/broken/forked/contradictory/unknown/ledger-mismatched history is
contextual `unreconstructable_memory_lifecycle`.

A scoped Observation with `created_at_epoch=NULL` is a contextual integrity
error because the public knowledge-time field is non-null; human-readable
`created_at` is not a canonical epoch fallback. This is covered separately from
`reference_time_epoch=NULL`, which validly falls back to the required creation
epoch.

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
evidence-less claim. External/pack remains Untrusted in mixed provenance.
Stored confidence never participates.

Captured events join canonical project. Expected project is nonblank
`memory.source_project`; only atomic unrouted legacy may use `memory.project`.
Missing/foreign/ambiguous/routed-without-source refs and malformed/dangling IDs
error. A present `source_candidate_id` is a claimed binding and must resolve a
completed `auto_promoted|approved|edited` candidate with exact evidence,
content/type/topic/confidence and an accepted input scope. Completion memory
scope must equal `CandidateRoute::memory_scope()` for the validated route:
`global` only for user owner, otherwise `project`; candidate scope and its
unpersisted derived title are not copied-equality fields. An exact
`memory_operation_log(source='memory_candidate',source_candidate_id=candidate.id,
result_memory_id=memory.id)` completion is required; workspace and pack fixtures
lock scope mapping/title exclusion. Owner/project routing may differ only by the
complete route-at-t chain defined above; scope is invariant. Chain-integrity
failures use `unreconstructable_routing_history`; other immutable drift is
`unverifiable_post_candidate_mutation`. Refs bind at candidate creation;
completion/cleanup knowledge is separately reference-eligible. Without a
candidate, a compatible durable result operation binds
refs. With neither claimed candidate nor operation, current `as_of=None` binds
refs at `reference_epoch`; explicit history excludes/Unknown. Malformed claimed
links error. Event source/insertion must not exceed binding/reference.
## Temporal Rules

- Define `effective_memory_knowledge_epoch` once for every memory ClaimView,
  SourceRef check and SourceTrustClass view. Proof is a validated candidate
  completion plus route chain, or a `memory-operation-planner-v1`
  `add|update|conflict` row exact-matching result ID/current canonical
  owner/type/topic; mismatches and other planners are non-proofs. A later
  canonical `noop` requires that planner, result ID/current owner/type, empty
  transitions, `noop_reason='already represented by active memory'`, and source tuple
  `direct/save_memory/NULL` or exact `memory_candidate/memory_candidate`
  plus matching noop candidate; input/result topics may differ. It proves only
  the transition.
  Knowledge is the max of earliest ingestion proof, eligible noops, memory
  update, candidate completion/ack, route events and complete/current memory
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

Claims, raw edge validation, captured evidence, facts, suppressions and
memory-governance/scope-cleanup events use scoped IDs/owners/projects and
existing indexes, never unrelated scans plus `Vec::contains`.
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
  and no unrelated row is returned/materialized;
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

Use five warmups and 50 measured runs. JSON schema version 1 includes exact
head SHA, seed/counts, chunk size, Rust/SQLite versions, plans, data/transaction
statement, bind and row counts, serialized bytes, p50/p95, fingerprints and
structural booleans. The Rust test validates it; latency stays informational.
Any later commit/base sync invalidates the record.
## Versioning and Golden Diff

v2 ships in 0.7.0 or the next explicit breaking boundary. Sync Cargo, lockfile,
plugin, runtime release manifest, npm wrapper, `server.json`, changelog, README
and architecture docs. The migration guide explicitly covers ClaimView
`created_at_epoch`/`updated_at_epoch` becoming version-specific
`source_time_epoch`/`knowledge_time_epoch`, including edit transitions,
in-place mutation limits and recency using the selected ClaimView’s effective
knowledge epoch.

`tests/truth_public_api.rs` acts as an external crate and compiles
`use remem::truth::{...}` for Project and Owner queries.

Allowed v1→v2 golden differences:

1. version, public entrypoint/export, TruthScope, typed subject/exact selector
   and effective epoch/replayability;
2. EvidenceView fields/integrity, ClaimView temporal-field replacement, and
   Observation catalog/read-scan/nullable-epoch/trust/attachment;
3. NULL/exact-empty singleton plus owner/scope/type isolation, canonical owner/target
   Project inclusion, stale non-repo placement exclusion, Owner memory+claim
   union, global/legacy fallback and user-claim-only compatibility wrapper;
4. versioned-edit and candidate multi-row/no-op reconstruction plus
   conservative in-place mutation handling;
5. suppression owner/time visibility;
6. canonical stored+recomputed source-trust cap, all-source binding-time
   checks, first-party explicit-user rules, candidate/result/edit invariants,
   summary-provenance fail-closed and full-blob escalation fix;
7. valid heterogeneous conflict error→neutral;
8. approved cross-topic output and overlapping-pair error;
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

Focused regressions cover governance before/equal/after plus broken/Web-ledger
chains; route before/equal/after for Project and Owner plus incomplete chains;
fact insertion-vs-learned time; and all six edge mappings plus unknown raw kind.
Also require WAL snapshot regression, final-head bounded/performance record,
fresh exact-head CI, independent review and human merge authorization.

## Phase Boundaries and Rollback

Phase B separately defines Context Bundle mapping, one shared render epoch,
worktree/task selectors, error-visible failure, budget/cache/historical output
and old-path rollback. Phase C separately decides writer convergence and any
migration/dual-write/backfill/cutover/firewall work.

Phase A v2 has no schema/backfill and its projection remains data-SELECT-only.
Roll back truth code, the duplicate-capture timestamp guard, tests, docs,
changelog and breaking-version metadata together. Published 0.6.x stays immutable.
