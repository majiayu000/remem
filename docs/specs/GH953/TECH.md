# Retrieval Engine Convergence — Tech Spec

Issue: #953 (parent #942)
Status: Current contract (stage S1 implemented; issue remains open)

## Current state

Two channel-assembly implementations exist.

`src/context/hybrid_context.rs` (injection):

- Private constants `RRF_K`, `MAX_VECTOR_DISTANCE`, `FTS_WEIGHT`,
  `ENTITY_WEIGHT`, `TEMPORAL_WEIGHT`, `FACT_WEIGHT`, `VECTOR_WEIGHT`,
  `LIKE_FALLBACK_WEIGHT`, `MIN_HYBRID_FETCH_LIMIT`.
- Channels: `fts`, `entity`, `temporal`, `fact`, `vector`; `like_fallback` only
  when every other channel is empty.
- Filters by `project`, `current_branch`, `excluded_types`, then resolves ids
  through `query_owner_included_memories_by_ids`.
- Fuses with `weighted_ranked_fuse(&channels, RRF_K)`.

`src/retrieval/search/memory/text.rs::build_query_search_plan` (search + eval):

- Reads every constant from `SearchWeights`.
- Channels: `fts`, `entity`, `fact`, `temporal`, `vector`, `graph`,
  `like_fallback`, `usage` (gated on `weights.usage > 0.0`).
- Filters by `project`, `memory_type`, `branch`, `include_stale`,
  `include_suppressed`; applies `min_evidence_confidence` in
  `gate_and_annotate_memories`.
- Emits `NamedChannel` with `disabled_reason` and `candidates_scanned`, which
  feeds `explain`.

The injection path is therefore missing `graph`, `usage`, and the evidence
confidence gate, and cannot observe any `SearchWeights` change.

### The duplication is deeper than the constants

Issue #953 describes the drift as copied constants. Reading the code, the
constants are the visible symptom of a larger fork:

- `hybrid_context.rs` carries nine private channel implementations with their
  own SQL — `query_local_fts_channel`, `query_local_entity_channel`,
  `query_local_entity_exact_channel`, `query_local_entity_like_channel`,
  `query_local_entity_ids`, `query_local_temporal_channel`,
  `query_local_fact_channel`, `query_local_vector_channel`,
  `query_local_like_channel` — filtered through
  `push_context_memory_filters(project, branch, excluded_types)`.
- The search path uses `memory::search_memories_*` helpers filtered by
  `memory_type`, `include_stale`, and `include_suppressed`.

The two filter vocabularies are not interchangeable, so convergence means
unifying a SQL layer, not hoisting six `const` items.

Post-fusion the paths diverge again. Search runs `load_ordered_memories` →
`source_anchor::apply_score_demotions` → `gate_and_annotate_memories` →
`apply_rerank_stage` → `paginate_memories`. Injection runs
`query_owner_included_memories_by_ids` and stops. Adopting the search pipeline
wholesale would silently add demotion, confidence gating, and reranking to
injection — three ranking-visible changes that have never run on that path.

This is why the work is staged below rather than landed as one change.

## Target design

### `RetrievalProfile`

New type in `src/retrieval/search/profile.rs`:

```rust
pub struct RetrievalProfile {
    pub weights: SearchWeights,
    pub channels: ChannelMask,
    pub candidate_multiplier: i64,
    pub min_candidate_pool: i64,
    pub scope: ScopeFilter,
}
```

`ChannelMask` is an explicit per-channel `bool` struct, not a bitflag set, so
adding a channel to the engine forces every profile constructor to name it —
this is the compile-time half of the success criterion.

`ScopeFilter` unifies what the two callers currently express differently:

```rust
pub struct ScopeFilter<'a> {
    pub project: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub memory_type: Option<&'a str>,
    pub excluded_types: &'a [String],
    pub include_stale: bool,
    pub include_suppressed: bool,
    pub owner_included_only: bool,
}
```

`owner_included_only` is the one genuinely injection-specific behavior: the
injection path resolves final ids through `query_owner_included_memories_by_ids`
rather than `load_ordered_memories`. It stays a scope flag rather than a second
code path.

Constructors:

- `RetrievalProfile::search()` — today's `build_query_search_plan` behavior,
  every channel enabled, `candidate_multiplier = 3`.
- `RetrievalProfile::injection()` — `candidate_multiplier = 3`,
  `min_candidate_pool = 20` (today's `MIN_HYBRID_FETCH_LIMIT`),
  `owner_included_only = true`.

### Engine

`build_query_search_plan` moves to `src/retrieval/search/engine.rs` and takes
`(conn, query_text, limit, offset, &RetrievalProfile)`. Every current channel
block gains a `if profile.channels.<name>` guard; a masked-off channel pushes
`NamedChannel::disabled(name, weight, "disabled by profile")` so `explain`
still accounts for it instead of the channel vanishing silently.

`hybrid_context::query_hybrid_context_memories` becomes a thin caller: build the
injection profile, call the engine, return the memories. Its private constants,
`ContextChannel`, and `push_channel` are deleted.

### Channel enablement for injection

The issue asks for one engine, not for a silent capability change. Initial
injection mask:

| Channel | Injection | Rationale |
|---|---|---|
| fts, entity, temporal, fact, vector | on | current behavior |
| like_fallback | on | current behavior (empty-channel fallback is engine-side) |
| graph | **on** | closing the documented gap is the point of the issue |
| usage | off | `weights.usage == 0.0` anyway; owned by #947 |

`min_evidence_confidence` gating is applied on the injection path too. Both the
`graph` enablement and the confidence gate are ranking-visible, so each must be
justified by the evaluation run below, and each is independently revertible by
flipping one mask field.

## Staging

Each stage is a separate PR with its own evaluation evidence. A stage that
shows an unexplained ranking delta stops the sequence.

| Stage | Scope | Injection output |
|---|---|---|
| S1 | `SearchWeights` becomes the only scoring-weight source for injection; delete the eight duplicated scoring `const` items in `hybrid_context.rs` while retaining injection-only candidate-pool depth. Channel SQL untouched. | byte-identical |
| S2 | Introduce `RetrievalProfile` / `ChannelMask` / `ScopeFilter`; injection assembles channels through the profile but keeps its own SQL behind the `ScopeFilter` translation. | byte-identical |
| S3 | Retire the nine `query_local_*` implementations one channel at a time in favour of the shared `memory::search_memories_*` helpers. One channel per commit, each proving identical ids on the fixture. | byte-identical per channel |
| S4 | Enable `graph` for injection. | ranking delta, evaluated |
| S5 | Apply `min_evidence_confidence` to injection. | ranking delta, evaluated |

`source_anchor` demotion and reranking on the injection path are explicitly out
of scope for #953; they change what users see for reasons unrelated to engine
convergence and belong to their own issues.

S1 proves that explicit `SearchWeights` values reach injection and that the
production wrapper uses the shipped defaults. It does not consume a generated
`eval-weight-grid` report, make that evaluator execute injection, or prove
shared-channel parity. Those issue-level acceptance criteria remain open until
the later engine and injection-evaluation stages.

## Verification

1. `channel_sources_are_shared` — asserts the channel name/weight pairs the
   engine produces under `injection()` and `search()` both come from the same
   `SearchWeights` instance, and that no `hybrid_context` symbol shadows a
   `SearchWeights` field.
2. `weight_change_reaches_injection` — runs injection twice over a fixture with
   two different `SearchWeights` values and asserts the returned ordering
   differs. This is the direct proof that weight-grid output governs injection.
3. `injection_channel_mask_is_exhaustive` — compile-time: `ChannelMask` has no
   `Default`, so every constructor must name every field.
4. Injection evaluation before and after, committed to the PR, showing no
   regression; the `graph` and confidence-gate deltas reported separately from
   the refactor.
5. `cargo test` green; no `const` in `hybrid_context.rs` duplicating a
   `SearchWeights` field (grep assertion in the test).

## Risks

- **Ranking drift disguised as refactor.** The two paths differ in filtering
  (`memory_type` vs `excluded_types`, suppression, staleness). If `ScopeFilter`
  translation is inexact, injection results change for reasons unrelated to the
  channel set. Mitigation: land the pure convergence with `graph` off and the
  confidence gate off first, prove byte-identical injection output on the
  fixture, then flip each of the two behavior switches in its own commit with
  its own evaluation delta.
- **`hybrid_context.rs` is 738 lines and `text.rs` is 710.** Moving the engine
  out of `text.rs` must not push either past the 800-line ceiling; the engine
  lands in its own module rather than being appended to an existing one.
