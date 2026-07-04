# Tech Spec

## Linked Issue

GH-717

## Product Spec

`product.md`

## Codebase Context

| Area | Files | Current behavior | Change |
| --- | --- | --- | --- |
| Observation dedup | `src/memory/dedup/funnel.rs`, `src/observation_extract.rs` | Hash dedup works; vector layer is a TODO and is not called from extraction persistence. | Add active-provider vector comparison over recent active observations and call it before inserting extracted observations. |
| Curated semantic dedup | `src/memory/semantic_dedup.rs`, `src/memory/store/write.rs`, `src/memory/operation.rs` | Query embeddings are active-provider embeddings and SQL filters by model/dimensions. | Keep same-model contract and clarify feature-hash threshold check. |
| Preference consolidation | `src/memory/preference/consolidation.rs` | Concept rules run first; embedding fallback calls legacy feature-hash helper and uses one threshold. | Use active `TextEmbedding` and model-specific thresholds for writes while keeping non-write text-only grouping on the local feature-hash path. |
| Tests | `src/memory/dedup/tests.rs`, preference tests, semantic dedup tests | Feature-hash and concept regressions exist, but active-model behavior is not directly covered. | Add focused tests for vector dedup and threshold/model guard behavior. |
| Specs | `docs/specs/local-semantic-embedding/`, `specs/GH682/` | Phase 4 is open. | Mark GH-717 behavior as landed in this PR while keeping GH-682 open for audit. |

## Proposed Design

Observation dedup keeps the existing hash stage as the first fast path.
`persist_observations` checks the funnel after exact replay idempotency and
before inserting an extracted observation. When no hash duplicate is found, the
funnel first scans recent active observations for the same project and returns
without embedding when there are no candidates. It then derives canonical
candidate text from `narrative`, `title + facts`, `facts`, or legacy
`observations.text`, embeds the incoming observation and each candidate through
the active provider with one shared fallback cache, recomputes the query
embedding if fallback changes the selected model/dimensions, compares only
same-space vectors, and marks matches accessed. Extracted observations perform
duplicate scoring before opening the batch write transaction; the transaction
still wraps the idempotency check and inserts. Provider `off` returns no vector
duplicates. Provider failures without an accepted fallback propagate because
silently comparing the wrong space would make duplicate decisions untrustworthy.
The vector stage keeps obvious opposite status updates such as passed/failed
separate.

Preference consolidation changes the embedding fallback from raw
`retrieval::vector::embed_query_text` to active `TextEmbedding` on the write
path. It returns before embedding when there are no active candidates, runs
concept classification across candidates first, and computes the incoming
embedding only when cosine fallback is needed. Candidate embeddings share the
same fallback cache as the incoming embedding so scoring stays in one model
space. `SamePreference` and `Contradiction` still win before cosine refinement;
weak concept `Refinement` matches continue into embedding scoring so a stronger
same-intent candidate can win. Provider errors propagate on the write fallback
path. The model threshold table is:

- `remem-local-feature-hash-v1`: `0.55`, preserving the #643 calibration.
- `fastembed-intfloat-multilingual-e5-small-v1`: `0.78`.
- `fastembed-bge-m3-v1`: `0.80`.
- OpenAI `text-embedding-3-*`: `0.82`.
- Unknown models: `0.90`, a conservative fail-closed default.

The existing `REMEM_PREF_EMBEDDING_THRESHOLD` test/ops override still works but
is interpreted as an explicit model-threshold override.

Curated-memory semantic dedup already embeds through
`retrieval::embedding::embed_memory` and filters by `e.model` plus
`e.dimensions`; this phase keeps that behavior and tightens tests/docs around
it rather than adding a second path.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 observation vector stage | `dedup/funnel.rs`, `observation_extract.rs` | Dedup test with paraphrased narratives plus extraction persistence wiring tests for narrative, title-only, fact-only, title+facts, and opposite-status observations |
| P2 active provider only | dedup and preference embedding helpers | Provider-off and model-threshold tests |
| P3 same-model curated dedup | `semantic_dedup.rs` | semantic_dedup focused test |
| P4 preference model thresholds | `preference/consolidation.rs` | threshold unit test |
| P5 polarity guards | preference and observation consolidation | existing and new contradiction/opposite-status regressions |
| P6 no silent provider degradation | helper error handling/logging | focused tests plus code review |

## Data Flow

Manual saves and candidate-promoted writes still enter through
`insert_memory_full` or `find_existing_memory`. Preference writes first check
topic/state keys, then active-model preference consolidation, then curated
semantic dedup. New memory embeddings are refreshed after insert/update using
the active provider and stored under that model id.

## Alternatives Considered

- Add an `observation_embeddings` table: rejected for this phase because GH-717
  can implement the funnel without a migration, and legacy observations are not
  the long-term capture ledger.
- Reuse the `0.55` threshold for every embedding provider: rejected by GH-717;
  feature-hash calibration does not transfer to local/API embeddings.
- Make render/audit text-only preference grouping call the active provider:
  rejected because those non-write surfaces should not perform live embedding
  calls while rendering or auditing. They keep the deterministic feature-hash
  fallback after concept rules and do not store duplicate decisions.

## Risks

- Performance: observation vector dedup computes embeddings over a bounded
  recent set. Keep the limit small and hash stage first.
- Provider availability: explicit local/API providers can fail during write
  consolidation; this is preferable to silently merging in the wrong space.
- Test isolation: env-var provider tests must hold the repo's test env lock.

## Test Plan

- [x] `cargo test dedup -- --test-threads=1`
- [x] `cargo test semantic_dedup -- --test-threads=1`
- [x] `cargo test preference -- --test-threads=1`
- [x] `cargo fmt --check`
- [x] `cargo check --message-format=short`
- [x] `cargo test -- --test-threads=1`
- [x] `python3 checks/check_workflow.py --repo . --spec-dir specs/GH682`
- [x] `python3 checks/check_workflow.py --repo . --spec-dir specs/GH717`

## Rollback Plan

Revert the funnel vector stage, preference active-embedding fallback, and tests.
Stored embeddings remain compatible because no schema changes are introduced.
