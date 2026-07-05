# Local Semantic Embedding Technical Spec

Status: Current contract
Date: 2026-07-04

Tracking:
- Epic issue: #682
- Design lineage: #358, #643
- Related contracts: #385, #675

## Existing Implementation Facts

- `src/retrieval/embedding.rs` defines `EmbeddingProvider { Auto, Local,
  FeatureHash, OpenAi, Off }` with `DEFAULT_PROVIDER = Auto`. `Auto` resolves
  to OpenAI when an API key is available (`REMEM_EMBEDDINGS_API_KEY` /
  `REMEM_EMBEDDINGS_API_KEY_ENV`), otherwise `FeatureHash` until GH-716
  commits eval evidence for a default flip.
- `Local` and `FeatureHash` are distinct provider states. `FeatureHash`
  produces `remem-local-feature-hash-v1`, 768-dim hashing-trick vectors.
  Explicit `Local` uses the verified local semantic runtime and reports
  unavailable when the configured model has not been downloaded.
- Config already reads a flat `[embeddings]` table from
  `~/.remem/config.toml` via `src/retrieval/embedding.rs::config_from_file()`,
  then applies `REMEM_EMBEDDINGS_PROVIDER`, `_FALLBACK`, `_MODEL`,
  `_BASE_URL`, `_DIMENSIONS`, `_API_KEY`, `_API_KEY_ENV`, `_MODEL_DIR`,
  `_TIMEOUT_SECS`.
- `memory_embeddings` stores blob + model id + dims with a multi-model primary
  key `(memory_id, model, dimensions)`, so a memory can carry feature-hash,
  local semantic, and API vectors concurrently.
- Vector channel weight is 3.0 with `MAX_VECTOR_DISTANCE = 0.51`
  (`src/retrieval/search/memory/weights.rs`); fusion is weighted RRF.
- GH-717 wires the observation dedup funnel into extraction persistence,
  adds active-provider vector dedup after the hash stage, and moves preference
  embedding fallback onto active-model embeddings with calibrated thresholds.
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
- The GH-715 runtime is `fastembed-rs` over ONNX Runtime, compiled behind the
  default-on `local-onnx` cargo feature. The shipped presets are
  `multilingual-e5-small` (default, 384 dimensions) and `bge-m3` (quality,
  1024 dimensions).
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

- Use `fastembed-rs` with ONNX Runtime, behind the default-on `local-onnx`
  cargo feature. Model weights are downloaded into
  `<model_dir>/<model-id>/`, not bundled in the release binary.
- Default preset: `multilingual-e5-small`, model id
  `fastembed-intfloat-multilingual-e5-small-v1`, upstream
  `intfloat/multilingual-e5-small`, 384 dimensions. Query inputs are prefixed
  with `query:` and memory/passages with `passage:`.
- Quality preset: `bge-m3`, model id `fastembed-bge-m3-v1`, upstream
  `BAAI/bge-m3`, 1024 dimensions.
- `remem embedding download [--model <preset>] [--json]` materializes the
  model and writes `remem-model-manifest.json` with schema version, runtime,
  dimensions, upstream model/source URL, file sizes, sha256 checksums, and
  hf-hub LFS source sha256 verification when the cache exposes a 64-hex source
  etag.
- `remem embedding status [--json]` reports installed models, verification
  state, model directory, and active-provider readiness.

### Embed Paths

- Write path: worker-side embedding on promotion and on `save_memory`,
  tagged with the active model id. If explicit `local` is configured but the
  model is missing or fails manifest verification, write paths log an error
  and defer the vector write instead of storing feature-hash under a local
  semantic model id.
- Query path: embed the query with the active model; if unavailable, fall
  back per config and mark the search result metadata as degraded so `why`
  output can explain ranking honestly.

### Backfill

- `remem embedding backfill [--batch N] [--limit N] [--prune] [--json]` embeds
  every searchable memory status that retrieval can expose for the active
  model, including stale and archived rows surfaced through explicit
  history/audit flags. It is idempotent, reports coverage at completion, and
  prunes other-model vectors only after coverage reaches 100% for that same
  searchable set and only with an explicit `--prune` flag.

### Tests

- Same-model-id guard test: mixed-model store never cross-scores.
- Backfill idempotency + prune-gating tests.
- Hook-path test: missing model defers embedding without blocking.

## Phase 3: Eval Gate

- Extend the golden set with paraphrase/synonym fixtures (EN + CJK) where
  feature-hash is known to fail.
- Run the retrieval gates for feature-hash / local semantic / API embeddings;
  commit reports under `eval/provider-comparison/` with model ids and
  thresholds.
- Default flip criteria (all required): paraphrase slice improves, no
  regression beyond gate thresholds on existing slices, p95 query embed
  latency within the search budget on a reference machine.
- Record the flip decision and evidence links in `docs/specs/README.md` index
  entry and the epic.

GH-716 records the reference command:

```bash
REMEM_DATA_DIR=eval/provider-comparison/reference-data \
  remem eval-provider-comparison --json-out eval/provider-comparison/report.json
```

The committed report keeps the default unchanged. `feature-hash` is available
and establishes the baseline, `local` is unavailable until the reference model
manifest is installed and verified, and `api` is unavailable in the committed
run because remote embedding calls require an explicit `--allow-api`.

## Phase 4: Downstream Adoption

- Observation dedup funnel: implemented the vector stage against the active
  semantic space and wired it into extraction persistence; thresholds are
  calibrated per model id, duplicate scoring happens before the extraction
  batch write transaction, and title+facts plus opposite-status regressions are
  covered (the 0.55 feature-hash preference threshold from #643 does not
  transfer automatically).
- Curated-memory semantic dedup: update the existing
  `src/memory/semantic_dedup.rs` call sites used by `save_memory`,
  `src/memory/store/write.rs`, and `src/memory/operation.rs` so manual and
  candidate-promoted memories use the same active-model semantics.
- Preference consolidation: same recalibration rule; keep the bidirectional
  polarity guard. GH-717 keeps the feature-hash preference threshold at its
  #643 calibration, uses stricter thresholds for local/API/unknown model ids,
  shares fallback state across write-path incoming/candidate embeddings, and
  keeps non-write text-only grouping on the deterministic feature-hash path so
  rendering and audit helpers do not perform live provider calls.

## Migration & Compatibility

- Existing migrations preserve old rows while replacing the
  single-row-per-memory constraint with `(memory_id, model, dimensions)`.
  Upsert and delete paths let a memory carry feature-hash, local semantic, and
  API vectors concurrently.
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

- Whether query-time embedding should cache recent query vectors for
  latency.

## Resolved Decisions

- GH-716 keeps `Auto` as "api if key else feature-hash"; local semantic
  embeddings remain explicit opt-in until committed comparison evidence
  includes verified local/API rows and release-platform readiness.
