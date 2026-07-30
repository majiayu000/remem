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

A memory owner pair must be complete. If both fields are absent, apply the
existing v019/default writer pair atomically: global becomes
`user/user:default`; otherwise `repo/memory.project`. A full nonblank pair is
authoritative even when a scope-cleanup reroute leaves `memories.project`
unchanged. Its stored `owner_scope` must be exactly one of `user`, `workspace`,
`repo`, `tool`, `domain`, `workstream`, `session`, and its stored owner key
must be trim-stable and nonblank. The normalized memory scope must be exactly
`project` or `global`; unknown owner/scope values and a partial/blank pair are
contextual integrity errors. Trimming memory scope is intentional v2
hardening: for example raw `" global "` is canonicalized to `global` and can
never leak into Project scope.

Project `branch=Some(B)` admits neutral + exact-B rows. `None` preserves the v1
all-branch/branch-agnostic behavior. Every relation endpoint must be in the
resulting scoped claim set.

Scope dispatch and inclusion are exact:

| Query scope | Included rows | Required validation |
| --- | --- | --- |
| Project(P,B) memories | normalized memory scope is not `global`, and either `owner_scope='repo' AND (owner_key=P OR target_project=P)` or atomic legacy owner NULL/NULL with `memories.project=P`; then (`B IS NULL` or row branch is NULL/exact B) | `source_project` is provenance only; a routed full owner pair is authoritative, and `memories.project`, `owner_key` and `target_project` may legitimately differ |
| Project(P,B) observations | canonical `projects.project_path = P` through `observations.project_id`, with the same branch rule | when `project_id` is NULL, a nonblank legacy `observations.project = P` is the only fallback; when both identities exist they must agree |
| Owner(S,K) memories | canonical owner after the atomic legacy fallback equals `(S,K)` | all memory branches are included because Owner has no branch selector; no Observation rows |
| Owner(S,K) user claims | `owner_scope = S AND owner_key = K` | exact pair only; no Observation rows |

This is the intentional v2 truth predicate, not a claim of byte-for-byte
equivalence with the legacy untrimmed context SQL. A repo-routed row can be
visible through `owner_key=P`,
`target_project=P`, or both; differing nonblank repo owner/target/placement
values are not corruption. The non-global guard applies to both full and
legacy owner arms; a repo-rerouted global remains Owner-only. A row rerouted to
`tool`, `domain`, `workstream`,
`session`, `workspace` or `user` is not selected merely because its stale
placement or target names P. It remains reachable through its exact Owner
query. Canonical and legacy global memories are likewise excluded from Project
and reachable through Owner (`user/user:default` for the atomic legacy
fallback). `source_project` only validates evidence provenance.

The scoped validation probe includes rows whose placement, repo owner or target
references P, and fails on partial owner pairs instead of silently omitting
them. For a full repo pair, blank target is treated as absent; owner/target
difference is legal. For a legacy pair, both owner fields must be NULL and the
fallback pair is derived atomically; `target_project` never expands legacy or
non-repo Project inclusion.

## Lifecycle and Observation Mapping

Keep existing lifecycle mappings and add:

| Source/status | publication | validity | retention | visibility |
| --- | --- | --- | --- | --- |
| observations/poisoning_quarantined | Candidate | Unknown | Live | Suppressed |

Actual memory, user-claim and Observation adapters validate their raw status
allowlists before projection. Unknown values return table/canonical-ref/raw
value context. A mapper’s generic Candidate/Unknown fallback is not a silent
adapter success.

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

The normal Observation writer is allowed to leave `evidence_event_ids=NULL`;
that value means an empty ref list. A non-null value must be a JSON integer
array; every ID must be positive, then sorted/de-duplicated and validated.
An empty array is valid. Every referenced event’s source and
`inserted_at_epoch` must be no later than the Observation creation binding and
query reference epochs, and its joined canonical `projects.project_path` must
equal the Observation’s canonical Project. If any Observation
`host_id/project_id/session_row_id` is present, all three are required, the
joined session must match them, and every event must exact-match the triple;
all-null is the explicit legacy path. Partial/dangling or cross-host/session
identity is an error, not retroactive/cross-project support.
Supporting events are reclassified canonically, so external backing makes the
Observation Untrusted rather than uplifting it to ModelGenerated.

Historical active rows do not carry a poisoning scan-version. Before an active
row can be marked Validated or emitted, the adapter re-scans its
title/subtitle/narrative/facts/concepts with the current generated-surface
scanner. A clean scan yields Validated. A hit on an active row returns a
contextual poisoning error and no successful projection; it may not be silently
treated as clean. A stored `poisoning_quarantined` row remains Quarantined and
is intentionally filtered as policy state without re-exposing its content.

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

Known fact status/predicate, replacement linkage and both scoped endpoint IDs
are validated. Event-validity is half-open; the final OR preserves the existing
bitemporal “not yet learned invalidation/replacement” behavior. Tests cover
learned/invalidation/replacement before, equal and after `t`.
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
lock scope mapping/title exclusion. Owner/project routing may differ only
through a contiguous, unambiguous `scope_cleanup` event chain whose
`previous_owner→new_owner` snapshots end at the current route; other immutable
or routing drift is `unverifiable_post_candidate_mutation`. Refs bind at
candidate creation; completion/cleanup knowledge is separately reference-
eligible. Without a candidate, a compatible durable result operation binds
refs. With neither claimed candidate nor operation, current `as_of=None` binds
refs at `reference_epoch`; explicit history excludes/Unknown. Malformed claimed
links error. Event source/insertion must not exceed binding/reference.
## Temporal Rules

- Define `effective_memory_knowledge_epoch` once for every memory ClaimView,
  SourceRef check and SourceTrustClass view. Proofs are a validated candidate
  completion (plus route chain) or `memory-operation-planner-v1`
  `add|update|conflict` rows exact-matching result ID and current canonical
  owner/type/topic; historical mismatches/other planners are non-proofs. After
  ingestion, a canonical `noop` needs that planner, result ID/current owner/type,
  empty transition sets, `noop_reason='already represented by active memory'`, and source tuple
  `direct/save_memory/NULL` or exact `memory_candidate/memory_candidate` with a
  matching noop candidate; input topic may differ from result topic. A noop is transition
  proof, never initial proof. Take the max of earliest ingestion proof, eligible
  noops, memory update, candidate completion/ack, route events and validated
  complete/current memory ack; partial/stale ack errors. No proof means historical exclusion/
  Unknown and current `reference_epoch`. Memory source time remains
  `COALESCE(reference_time_epoch, created_at_epoch)` and must be reference-
  eligible; impossible future raw timestamps error.
  A direct-save noop timestamps its same-transaction trust/ack rewrite by the
  operation; governance ack uses memory update, candidate ack its candidate update.
- A UserContextClaim source epoch is
  `COALESCE(valid_from_epoch,created_at_epoch)`. Edited descendants retain the
  provenance-root SourceRef binding above; transitions change ClaimView state
  knowledge but never reattach inherited refs.
- For a predecessor selected at `t < transition`, ClaimView serializes the
  predecessor’s pre-transition lifecycle and
  `knowledge_time_epoch=created_at_epoch`. At transition equality and later,
  the successor is current with source/knowledge derived from its own row; if
  the predecessor is retained as rejected provenance, its ClaimView uses the
  transition epoch and Superseded lifecycle. The predecessor’s mutated
  `updated_at_epoch` is therefore an edit boundary, not its immutable
  SourceRef knowledge time.
- For an in-place suppress/unsuppress/delete mutation, a query at or after the
  stored `updated_at_epoch` may serialize the current row with ClaimView
  `knowledge_time_epoch=updated_at_epoch`; a query before it cannot reconstruct
  that prior state and must exclude/return Unknown. SourceRef knowledge remains
  the provenance-root binding because those refs were not rewritten.
- Captured-event source is `COALESCE(reference_time_epoch, created_at_epoch)`;
  knowledge is original insertion. Replay of `(host_id, session_id, event_id)`
  preserves row timestamps/payload but may append keyed Git evidence/work. Pre-v2
  stored insertion is a conservative floor; source/knowledge must be `<=reference_epoch`.
- A one-to-one `edit_claim` chain uses the superseded old row before transition
  and the successor at/after transition. Missing/forked/cross-owner or
  timestamp-inconsistent explicit edit chains error.
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
- Suppress/unsuppress/delete mutate one row in place. If their
  `updated_at_epoch` is after the cutoff and no version row exists, exclude or
  return Unknown rather than back-project current state.
- Hard-deleted/general rewritten rows cannot be reconstructed without history;
  Phase A deliberately returns less rather than guessing.

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

Only relations with both endpoints in the scoped set are loaded.

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

For each output in an approved cross-topic pair, every field is fixed:
`subject` remains that slot’s identity; `claim=None`;
`validity=Contradicted`; `evidence` is the evidence-ref-deduplicated, standard-
sorted union from both survivors; `supporting_relations=[]`;
`contradicting_relations` is the standard-sorted set of validated canonical
conflict relations connecting the pair; `rejected` preserves that slot’s
already-rejected canonical refs, byte-sorted/deduplicated;
`conflicting_claims` contains both survivors in canonical-ref byte order; and
`selected_reason=UnresolvedConflict`. Both outputs use this same pair and
relation set.

Unbacked means both source IDs are NULL; it uses edge creation as knowledge
without lookup, and an unbacked conflict is decision-neutral. Candidate-only is
an integrity error. A source-operation ID starts bound provenance: edge and
operation must exist, be reference-eligible, and satisfy
`operation.created_at_epoch <= edge.created_at_epoch`. Operation-only is valid.
A claimed candidate must match the operation discriminator/ID, be created no
later than it, and prove canonical memory candidate status/result/operation/
endpoints or graph candidate status/`promoted_edge_id`/`source_operation_id`.
Writers finish candidate state after operation/edge insertion, so knowledge is
the max of their creation and validated application update; it must be
reference-eligible but need not precede edge creation. Dangling/future/mismatch
errors; unbacked, candidate-only, operation-only and application boundary
fixtures prevent provenance bypass or retroactive visibility.

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

Claims, edges, captured evidence, Observation links and suppressions use scoped
IDs/owners and existing indexes, never unrelated scans plus `Vec::contains`.
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

Also require a WAL concurrent-writer snapshot regression, final-head bounded/
performance record, fresh exact-head CI, independent review and human merge authorization.

## Phase Boundaries and Rollback

Phase B separately defines Context Bundle mapping, one shared render epoch,
worktree/task selectors, error-visible failure, budget/cache/historical output
and old-path rollback. Phase C separately decides writer convergence and any
migration/dual-write/backfill/cutover/firewall work.

Phase A v2 has no schema/backfill and its projection remains data-SELECT-only.
Roll back truth code, the duplicate-capture timestamp guard, tests, docs,
changelog and breaking-version metadata together. Published 0.6.x stays immutable.
