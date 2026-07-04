# Product Spec

## Linked Issue

GH-717

## User Problem

remem now records embedding provider state, supports local semantic vectors, and
commits provider-comparison evidence, but downstream dedup paths still have
gaps: observation dedup has no vector stage, and preference same-intent
consolidation can still use the old feature-hash threshold without checking the
active model id. That can miss paraphrases when semantic embeddings are active
or merge too aggressively when a higher-quality provider is selected.

## Goals

- Implement observation vector dedup in the active embedding provider/model
  space after the hash stage.
- Keep curated-memory semantic dedup on same-model vectors for manual saves and
  candidate-promoted memory writes.
- Move preference consolidation's embedding fallback onto active-model
  embeddings with model-specific thresholds.
- Preserve contradiction, exclusive language, and bidirectional polarity guards
  so semantic similarity cannot merge genuine opposites.
- Add focused regression coverage for paraphrase consolidation and unrelated or
  contradictory memories remaining separate.

## Non-Goals

- Do not change the default embedding provider; GH-716 recorded a no-flip
  decision until local/API comparison rows are available.
- Do not add ANN indexing or new vector tables for observations.
- Do not remove the deterministic feature-hash fallback.
- Do not close GH-682 from this issue unless the epic closure audit is also
  complete.

## Behavior Invariants

1. Observation dedup runs hash first, then a vector stage only when embeddings
   are enabled and available, using canonical observation text from `narrative`,
   `title + facts`, `facts`, or legacy `text`.
2. The vector stage compares embeddings produced by the active provider/model,
   not an implicit legacy helper detached from provider resolution.
3. Curated-memory semantic dedup filters candidates by the query embedding's
   model id and dimensions before cosine scoring.
4. Preference consolidation uses the active embedding provider for its fallback
   and applies a threshold calibrated for the returned model id.
5. Feature-hash keeps its existing evidence-backed preference threshold; local,
   API, and unknown real embedding models use stricter thresholds.
6. Provider `off` disables vector matching without warning; provider errors that
   would hide user-visible dedup behavior are returned or logged at error level.
7. Polarity and contradiction guards run before embedding fallback can merge
   memories.
8. Observation vector dedup does not collapse obvious opposite status updates
   such as passed/failed, and extraction persistence does not hold the batch
   write transaction open while waiting on live embedding calls.

## Acceptance Criteria

- [x] `src/memory/dedup/funnel.rs` has an active-provider vector stage after
      hash dedup, and observation extraction persistence calls the funnel
      before inserting a new observation.
- [x] Preference consolidation no longer calls the legacy feature-hash helper
      as a provider-independent fallback.
- [x] Preference embedding fallback thresholds vary by active model id.
- [x] Existing `save_memory`, `src/memory/store/write.rs`, and
      `src/memory/operation.rs` call sites share the updated consolidation and
      semantic-dedup behavior.
- [x] Regression tests cover paraphrase/same-intent consolidation and
      unrelated or contradictory content staying separate.
- [x] Verification includes focused dedup, semantic_dedup, and preference tests,
      plus fmt, check, and full test suite before merge readiness.

## Edge Cases

- Active provider is `off`: skip vector matching and leave hash/concept rules
  intact.
- Explicit local provider has no verified model: write-path semantic matching
  returns the provider error instead of silently using feature-hash as local.
- Unknown API model id: use a conservative threshold rather than the feature-hash
  threshold.
- Existing stored feature-hash vectors remain valid under their model id and are
  only compared with feature-hash queries.

## Rollout Notes

Use `Closes #717` and `Refs #682` in the implementation PR. GH-682 remains open
until all phase links, eval evidence, spec index updates, and final closure
audit are complete.
