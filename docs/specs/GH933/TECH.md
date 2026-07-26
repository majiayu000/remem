# GH933 Tech Spec — CurrentTruth Read-Side Projection (Phase A)

Refs #933. See `PRODUCT.md` for scope; this file is the implementation
contract.

## Module Layout

```
src/truth.rs              module root, projection version constant
src/truth/types.rs        versioned read DTOs + lifecycle enums
src/truth/lifecycle.rs    stored-status -> Lifecycle mapping (per source table)
src/truth/adapter.rs      read-only SQL mapping from existing tables
src/truth/projection.rs   deterministic resolution policy
src/truth/tests.rs        golden fixtures (in-memory DB + run_migrations)
```

All files stay under 800 lines. The module is read-only: it takes a
`&rusqlite::Connection`, issues SELECTs only, and never writes.

## DTOs (all `Serialize`, all carry `projection_version = 1`)

- `EvidenceView { evidence_ref, kind, source_ref, observed_at_epoch, trust }`
  - `EvidenceKind`: `CapturedEvent`, `SourceRef` (claim source_refs entries).
  - `EvidenceTrust`: `Verified` (user-authored or tool/repo-verifiable),
    `ModelGenerated`, `Untrusted` (external content).
- `ClaimView { canonical_ref, source, subject_key, statement, scope,
  branch, lifecycle, visibility, valid_from/valid_to, created/updated,
  evidence: Vec<EvidenceView> }`
  - `ClaimSource`: `Memory` (`memory:<id>`), `UserContextClaim`
    (`user_claim:<id>`). Observations and candidates are lifecycle-mapped but
    are not claim sources in Phase A (candidates are unpublished; generated
    enrichment never creates a claim).
- `RelationView { relation_ref, kind, from_ref, to_ref, created_at_epoch,
  valid_from/valid_to }` with `ClaimRelationKind`:
  `Supersedes`, `Refutes`, `Supports`, `DerivedFrom`, `AppliesTo`.
- `CurrentTruthView { claim, validity, evidence, supporting_relations,
  contradicting_relations, selected_reason, rejected: Vec<canonical_ref> }`
- `CurrentTruthProjection { projection_version, project, branch, as_of_epoch,
  truths, abstentions }`.

## Lifecycle Mapping (deterministic, total per source)

Three orthogonal dimensions + visibility:

- `PublicationState`: `Candidate | Reviewed | Active`
- `ValidityState`: `Current | Superseded | Contradicted | Stale | Expired | Unknown`
- `RetentionState`: `Live | Archived | Deleted`
- `Visibility`: `Visible | Suppressed` (policy suppression is not falsity)

| Source | Stored status | publication | validity | retention | visibility |
|---|---|---|---|---|---|
| memories | active | Active | Current | Live | Visible |
| memories | stale | Active | Stale | Live | Visible |
| memories | superseded | Active | Superseded | Live | Visible |
| memories | archived | Active | Current | Archived | Visible |
| memories | deleted | Active | Unknown | Deleted | Visible |
| memories | rejected | Candidate | Unknown | Deleted | Visible |
| observations | active | Active | Current | Live | Visible |
| observations | stale | Active | Stale | Live | Visible |
| observations | compressed | Active | Current | Archived | Visible |
| user_context_claims | active | Active | Current | Live | Visible |
| user_context_claims | pending_review | Candidate | Unknown | Live | Visible |
| user_context_claims | stale | Active | Stale | Live | Visible |
| user_context_claims | superseded | Active | Superseded | Live | Visible |
| user_context_claims | suppressed | Active | Current | Live | Suppressed |
| user_context_claims | rejected | Candidate | Unknown | Deleted | Visible |
| user_context_claims | deleted | Active | Unknown | Deleted | Visible |
| memory_candidates | pending_review | Candidate | Unknown | Live | Visible |
| memory_candidates | quarantined | Candidate | Unknown | Live | Suppressed |
| memory_candidates | deferred | Candidate | Unknown | Live | Visible |
| memory_candidates | failed | Candidate | Unknown | Live | Visible |
| memory_candidates | discarded | Candidate | Unknown | Deleted | Visible |
| memory_candidates | auto_promoted | Active | Current | Live | Visible |
| memory_candidates | approved | Reviewed | Current | Live | Visible |
| memory_candidates | accepted | Reviewed | Current | Live | Visible |
| memory_candidates | edited | Reviewed | Current | Live | Visible |

Unknown stored strings map to `(Candidate, Unknown, Live, Visible)` and are
never eligible as current truth (fail closed, no panic on legacy data).
`Expired` is derived, not stored: `valid_to_epoch`/`expires_at_epoch` earlier
than the reference time overrides validity to `Expired`.

## Adapter (read-only)

- Claims from `memories` filtered by `project` (+ `target_project`
  fallthrough is out of scope in Phase A; exact `project` match only) and
  optional branch (`branch IS NULL OR branch = ?`), grouped by `topic_key`
  (NULL topic_key = singleton group keyed `memory:<id>`).
- Claims from `user_context_claims` grouped by `(claim_type, claim_key)` when
  an owner selector is supplied via `TruthQuery.user_owner`.
- Relations from `memory_edges` (`supersedes` -> Supersedes, `conflicts` ->
  Refutes, `derived_from`/`extracted_from` -> DerivedFrom, `duplicates`/
  `merged_into`/`split_from` -> Supports-adjacent are mapped DerivedFrom;
  diagnostic-only edges are ignored), from trusted `graph_edges`
  memory-to-memory rows, and from `user_context_claims.supersedes_claim_id`.
  Stored `memory_edges` replacement rows are `(from=old, to=new)`; the DTO
  normalizes every Supersedes relation to "`from_ref` supersedes `to_ref`".
- Evidence: `memories.evidence_event_ids` resolved against `captured_events`
  (`role='user'` -> Verified, `tool_name` present -> Verified, otherwise
  ModelGenerated), `memories.source_trust_class`
  (`user_prompt`/`repo_file`/`local_tool_output` -> Verified,
  `external_content` -> Untrusted) as a claim-level evidence floor, and claim
  `source_refs_json` entries (`source_kind` containing `llm`/`model`/
  `summary` -> ModelGenerated, else Verified).

## Projection Policy (strict order, first hit wins)

For each subject group after scope/branch filtering:

1. Drop rows created after `as_of`; clamp validity windows to `as_of`
   (falls back to "now" when `as_of` is absent).
2. Drop retention `Deleted`; retention `Archived` never enters current truth
   (still legal for historical explanation, out of Phase A output).
3. Drop publication `Candidate` and visibility `Suppressed` rows.
4. Apply explicit Supersedes relations effective at the reference time:
   superseded side is removed and recorded in `rejected`.
5. Drop validity `Stale`/`Expired` rows. If nothing remains -> abstention
   (`Unknown`, reason `InsufficientEvidence`).
6. One row left -> `Current`, reason `OnlySurvivingClaim` (or
   `ExplicitSupersedes` when step 4 removed a competitor).
7. Multiple rows: if any Refutes relation connects two survivors ->
   `Contradicted`, reason `UnresolvedConflict`, both sides attached.
8. Otherwise prefer the row whose evidence tier is strictly better
   (Verified > ModelGenerated > Untrusted), reason
   `VerifiedEvidencePreferred`; tie on tier resolves by newest
   `updated_at_epoch`, reason `MostRecent`; an exact timestamp tie is
   `Contradicted` (no arbitrary winner).

Determinism: all inputs ordered by `(epoch, id)`; no randomness, no LLM, no
stored confidence in the decision path.

## as_of Limitation (documented, accepted for Phase A)

Stored statuses have no history table; `as_of` reasons over temporal columns
(`created_at_epoch`, `valid_from/valid_to`, relation `created_at_epoch`), so a
row hard-deleted or status-rewritten in place after the fact cannot be
perfectly reconstructed. Supersedes relations are timestamped, so the primary
"decision was replaced later" history case works and is fixture-covered.

## Tests

`src/truth/tests.rs` builds an in-memory DB via `crate::migrate::run_migrations`
and covers: explicit supersedes; two-evidence support; refutes-conflict ->
Contradicted; verified-vs-model preference; branch isolation; `as_of`
historical query returning the then-current, now-superseded decision; scope
(project) isolation; abstention on empty/stale-only groups; serialized DTO
shape lock for one golden case. `src/truth/lifecycle.rs` unit tests cover
every stored status value in the mapping table above.

## Verification

```
cargo fmt --check
cargo clippy -- -D warnings
cargo test truth
cargo test
```
