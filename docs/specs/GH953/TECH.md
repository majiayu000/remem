# Retrieval Engine Convergence — Tech Spec

Issue: #953 (parent #942)
Status: Current contract (stage S1 implemented; issue remains open)

## Pre-S1 state

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

The injection path was therefore missing `graph`, `usage`, and the evidence
confidence gate, and could not observe any `SearchWeights` change. S1 replaces
only the private scoring constants with an explicit `SearchWeights` input; the
remaining differences stay open below.

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

### #954 rank-signal correction

The S1 byte-identical requirement applied to centralizing weight ownership. A
later correctness finding in #954 identified that rank-only channel hits were
also converted to `normalized_score = 1 / (rank + 1)`, so rank influenced both
the RRF denominator and a synthetic signal multiplier. #954 removes that
double-counting in both search and injection while leaving calibrated FTS and
vector signals intact.

For injection, `eval-injection` owns a discriminating two-arm fixture. Both arms
call `query_hybrid_context_memories_with_rank_signal_mode` with the same database,
query, filters, channel SQL, weights, and result limit. The baseline arm applies
only the legacy rank pseudo-score conversion; the candidate arm uses pure
weighted RRF for rank-only hits. The gate requires the expected memory to move
from rank 2 to rank 1 without regressing MRR@10 or nDCG@10. This is evidence for
the #954 behavior change, not completion evidence for #953's shared-engine,
channel-parity, graph, or confidence-gate stages.

S1 proves that explicit `SearchWeights` values reach injection and that the
production wrapper uses the shipped defaults. It does not consume a generated
`eval-weight-grid` report, make that evaluator execute injection, or prove
shared-channel parity. Those issue-level acceptance criteria remain open until
the later engine and injection-evaluation stages.

## Verification

S1 implements nine focused tests, grouped by guarantee:

1. `injection_ordering_follows_search_weights` plus the FTS/entity, temporal,
   fact, LIKE fallback, vector/distance, and RRF-k focused tests make all eight
   injection scoring fields individually observable on the retrieval/fusion
   path. This proves only that explicit weights reach injection; it does not
   prove that `eval-weight-grid` executes injection or that its generated
   report is applied at runtime.
2. `default_weights_are_the_production_path` — proves the zero-argument
   production wrapper is equivalent to explicit `SearchWeights::default()`.
3. `hybrid_context_declares_no_private_scoring_constants` — rejects all eight
   former scoring `const` declarations in `hybrid_context.rs`.

The issue-level completion verification remains:

1. `channel_sources_are_shared` — assert the channel name/weight pairs produced
   under `injection()` and `search()` come from one engine/profile definition.
2. `injection_channel_mask_is_exhaustive` — compile-time: `ChannelMask` has no
   `Default`, so every constructor must name every field.
3. Run injection evaluation before and after, commit the evidence to the PR,
   and report the `graph` and confidence-gate deltas separately from the
   behavior-preserving refactor.
4. Keep `cargo test` green through every stage.

Separately, #954 must keep the injection rank-signal A/B green; its baseline and
candidate are evaluated on the actual injection channel assembly and serialized
in the `eval-injection` report.

## Risks

- **Ranking drift disguised as refactor.** The two paths differ in filtering
  (`memory_type` vs `excluded_types`, suppression, staleness). If `ScopeFilter`
  translation is inexact, injection results change for reasons unrelated to the
  channel set. Mitigation: land the pure convergence with `graph` off and the
  confidence gate off first, prove byte-identical injection output on the
  fixture, then flip each of the two behavior switches in its own commit with
  its own evaluation delta.
- **`hybrid_context.rs` is 767 lines and `text.rs` is 710.** Moving the engine
  out of `text.rs` must not push either past the 800-line ceiling; the engine
  lands in its own module rather than being appended to an existing one.
