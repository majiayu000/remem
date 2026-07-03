# Local Semantic Embedding Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Epic issue: #682
- Design lineage: #358, #643
- Related contracts: #385, #675

## Existing Implementation Facts

- `src/retrieval/embedding.rs` defines `EmbeddingProvider { Auto, Local,
  OpenAi }` with `DEFAULT_PROVIDER = Auto`. `Auto` resolves to OpenAI when an
  API key is available (`REMEM_EMBEDDINGS_API_KEY` /
  `REMEM_EMBEDDINGS_API_KEY_ENV`), otherwise `Local`.
- `Local` produces `remem-local-feature-hash-v1`, 768-dim hashing-trick
  vectors. `"local" | "offline" | "feature-hash" | "feature_hash"` all parse
  to the same variant, so "local" and "feature-hash" are currently the same
  thing.
- Config already reads a flat `[embeddings]` table from
  `~/.remem/config.toml` via `src/retrieval/embedding.rs::config_from_file()`,
  then applies `REMEM_EMBEDDINGS_PROVIDER`, `_MODEL`, `_BASE_URL`,
  `_DIMENSIONS`, `_API_KEY`, `_API_KEY_ENV`, `_TIMEOUT_SECS`.
- `memory_embeddings` (v029) stores blob + model id + dims, but
  `memory_id INTEGER PRIMARY KEY` currently allows only one vector row per
  memory. Multi-model coexistence requires a schema migration before the
  default can flip.
- Vector channel weight is 3.0 with `MAX_VECTOR_DISTANCE = 0.51`
  (`src/retrieval/search/memory/weights.rs`); fusion is weighted RRF.
- The dedup funnel has one open TODO for vector-based dedup
  (`src/memory/dedup/funnel.rs`); preference consolidation uses
  embedding-cosine fallback in the feature-hash space (#643).
- Eval surfaces: `eval/golden.json`, `eval/gates/`, `remem eval` /
  `eval-local` harness.

## Design Rules

- `Local` must stop being an alias of feature-hash. The provider enum gains a
  distinct semantic-local variant; `feature-hash` parses to its own variant.
- Cosine comparison only within one model id. Query embedding uses the active
  model; candidate set is filtered by model id before scoring.
- No silent degradation (U-29): resolved-provider != configured-provider is
  an error-level log plus a status/doctor surface, never a quiet fallback.
- Model weights are never bundled; the default download target is derived
  from `REMEM_DATA_DIR` (`<data-dir>/models/<model-id>/`) with checksum
  verification so eval and smoke runs never touch a real user's home data.
- Hook latency budget: hooks must never block on model download. If the
  active model is unavailable inside a hook path, embedding work defers to
  the worker; hooks write no vectors rather than wrong vectors.
- All default changes are gated on committed eval artifacts.

## Phase 1: Provider Contract

### Config

Extend the existing `[embeddings]` section in `~/.remem/config.toml`; do not
introduce a second singular `[embedding]` namespace:

```toml
[embeddings]
provider = "local"        # api | local | feature-hash | off
fallback = "feature-hash" # optional; omit for fail-closed
model = ""                # optional override per provider
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
model_dir = ""            # default: <REMEM_DATA_DIR>/models
```

Resolution order: CLI/env override > config file > built-in default.
Env variables keep their current names and win over the file for
automation compatibility.

`provider = "off"` is an explicit disabled state, not a degraded fallback:
search skips query embedding and vector fusion, write/backfill paths do not
write vectors, `status --json` reports `active_provider = "off"` and
`disabled = true`, and `doctor` does not warn about vector coverage. Stale
vectors from an earlier provider remain stored but are ignored until a
non-off provider is selected.

### Visibility

- `remem status --json` gains an `embedding` object: configured provider,
  active provider, active model id, degraded flag, vector coverage
  (`embedded/total` for the active model).
- `remem doctor` adds findings: configured provider unavailable; coverage
  below threshold for the active model; mixed-model vectors present without
  a completed backfill.

### Tests

- Config parse + resolution-order tests.
- Degraded-state test: provider=api without key resolves to fallback, status
  reports degraded, log line at error level.

## Phase 2: Local Semantic Model

### Runtime

- Add an ONNX runtime dependency behind a cargo feature that is on for
  release builds; evaluate `ort` vs `fastembed-rs` class integration and
  record the choice plus binary-size impact in the epic.
- Default preset: a small multilingual sentence-embedding model with strong
  CJK+EN behavior. Candidate presets: multilingual-e5-small class
  (~120MB int8) as default, bge-m3 class as a quality preset. Final choice
  is an epic decision recorded here before implementation.
- `remem embedding download [--model <preset>]` fetches weights with
  checksum + resume; `remem embedding status` reports installed models.

### Embed Paths

- Write path: worker-side embedding on promotion and on `save_memory`,
  tagged with the active model id.
- Query path: embed the query with the active model; if unavailable, fall
  back per config and mark the search result metadata as degraded so `why`
  output can explain ranking honestly.

### Backfill

- `remem embedding backfill [--batch N]`: embeds every searchable memory
  status that retrieval can expose for the active model, including stale and
  archived rows surfaced through explicit history/audit flags. It is
  idempotent, reports coverage at completion, and prunes other-model vectors
  only after coverage reaches 100% for that same searchable set and only with
  an explicit `--prune` flag.

### Tests

- Same-model-id guard test: mixed-model store never cross-scores.
- Backfill idempotency + prune-gating tests.
- Hook-path test: missing model defers embedding without blocking.

## Phase 3: Eval Gate

- Extend the golden set with paraphrase/synonym fixtures (EN + CJK) where
  feature-hash is known to fail.
- Run the retrieval gates for feature-hash / local semantic / API embeddings;
  commit reports under `eval/gates/` with model ids and thresholds.
- Default flip criteria (all required): paraphrase slice improves, no
  regression beyond gate thresholds on existing slices, p95 query embed
  latency within the search budget on a reference machine.
- Record the flip decision and evidence links in `docs/specs/README.md` index
  entry and the epic.

## Phase 4: Downstream Adoption

- Observation dedup funnel: implement the vector stage against the active
  semantic space; thresholds calibrated per model id (the 0.55 feature-hash
  threshold from #643 does not transfer automatically).
- Curated-memory semantic dedup: update the existing
  `src/memory/semantic_dedup.rs` call sites used by `save_memory`,
  `src/memory/store/write.rs`, and `src/memory/operation.rs` so manual and
  candidate-promoted memories use the same active-model semantics.
- Preference consolidation: same recalibration rule; keep the bidirectional
  polarity guard.

## Migration & Compatibility

- Add a migration that preserves existing rows while replacing the
  single-row-per-memory constraint with a multi-model key such as
  `(memory_id, model, dimensions)`. Update upsert and delete paths in the
  same PR so a memory can carry feature-hash, local semantic, and API vectors
  concurrently.
- Existing feature-hash vectors stay valid under their model id until the
  user backfills and prunes.

## Verification

```bash
cargo fmt --check
cargo check
cargo test
remem eval-local   # retrieval gates with committed thresholds
```

## Open Questions

- ONNX runtime choice and its effect on build matrix (musl/windows targets).
- Whether query-time embedding should cache recent query vectors for
  latency.
- Whether `Auto` remains "api if key else local" after local becomes truly
  semantic, or is retired in favor of explicit provider selection.
