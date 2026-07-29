# Local Semantic Embedding Technical Spec

Status: Current contract
Date: 2026-07-28

Tracking:
- Epic issue: #682
- Design lineage: #358, #643
- Conditional auto-activation: #946
- Related contracts: #385, #675

## Existing Implementation Facts

- `src/retrieval/embedding.rs` defines `EmbeddingProvider { Auto, Local,
  FeatureHash, OpenAi, Off }` with `DEFAULT_PROVIDER = Auto`. `Auto` resolves
  to OpenAI when a remem-specific API key is available
  (`REMEM_EMBEDDINGS_API_KEY` / custom
  `REMEM_EMBEDDINGS_API_KEY_ENV`), otherwise to a verified installed
  `multilingual-e5-small`, otherwise `FeatureHash`. The default
  `OPENAI_API_KEY` environment name alone does not opt `Auto` into remote
  calls.
- `Local` and `FeatureHash` are distinct provider states. `FeatureHash`
  produces `remem-local-feature-hash-v1`, 768-dim hashing-trick vectors.
  Explicit `Local` uses the verified local semantic runtime and reports
  unavailable when the configured model has not been downloaded.
- Config already reads a flat `[embeddings]` table from
  `~/.remem/config.toml` via
  `src/retrieval/embedding/config.rs::config_from_file()`,
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
- Provider-selection changes must carry committed eval evidence. They must not
  trigger model downloads from hooks or searches.

## Phase 1: Provider Contract

### Config

Extend the existing `[embeddings]` section in `~/.remem/config.toml`; do not
introduce a second singular `[embedding]` namespace:

```toml
[embeddings]
provider = "auto"         # auto | api | local | feature-hash | off
fallback = "feature-hash" # optional; omit for fail-closed
model = "text-embedding-3-small"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
model_dir = ""            # default: <REMEM_DATA_DIR>/models
```

Resolution order: CLI/env override > config file > built-in default.
Env variables keep their current names and win over the file for
automation compatibility.

When the configured provider is `auto`, runtime resolution is:

1. a remem-specific API key;
2. the verified default local model, if its manifest and runtime are ready;
3. feature-hash when no local install exists.

An existing but invalid local installation is not treated like absence:
status/doctor expose a degraded feature-hash activation with the verification
or runtime error.

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
  with `query:` and memory/passages with `passage:`. Since GH-850, memory
  passages for enrichment-ready rows append the row's index-only
  `search_context` snapshot (`memory-index-v2` passage schema);
  `memory_embeddings.content_hash` is the versioned SHA-256 of that passage,
  so FTS and the vector channel provably consume the same enrichment
  snapshot. Pending rows keep the canonical-only passage, which preserves the
  pre-existing paraphrase recall and curated semantic-dedup comparisons.
- Quality preset: `bge-m3`, model id `fastembed-bge-m3-v1`, upstream
  `BAAI/bge-m3`, 1024 dimensions.
- `remem embedding download [--model <preset>] [--json]` materializes the
  model and writes schema-v2 `remem-model-manifest.json` with runtime,
  dimensions, upstream model/source URL, regular-file sizes and SHA-256
  checksums, plus every active HF snapshot symlink's exact relative target and
  resolved blob. hf-hub LFS source SHA-256 is also verified when the cache
  exposes a 64-hex source etag.
- A per-model download lock serializes downloads, but network transfer and the
  verified readiness probe run in a fresh owner-only staging cache without
  blocking active model readers. Shipped presets accept only the official
  Hugging Face endpoint. The default E5 preset must match the evaluated,
  layout-independent content digest before any ONNX session is constructed.
  Immutable no-clobber artifact import completes before the short state lock;
  the state lock protects only active-ref/manifest activation and journal
  recovery. A synced `Prepared`/`Committing` journal makes the active
  revision plus manifest transition crash-recoverable, so readers see either
  the previous verified model or the completed candidate. Download and both
  local runtimes ignore unrelated `HF_HOME`: E5 is loaded from verified owned
  bytes, while BGE uses a remem-owned file-backed ORT session over the verified
  private snapshot. Private-cache disappearance fails loudly instead of
  invoking hf-hub or downloading a replacement.
  Unix staging/cache directories and locks use owner-only modes plus
  no-follow identity checks. Windows local-model operations support only the
  default per-user model root; `embeddings.model_dir` and `REMEM_DATA_DIR`
  overrides fail closed. Windows creates the root, install, staging/cache, and
  lock objects with protected current-owner-only DACLs, rejects reparse
  points, and compares the full volume serial plus 128-bit `FILE_ID_INFO`
  before trusting a reopened path. Stable directories and locks retain
  no-delete-share anchors. Renameable staging uses an identity-bound handle,
  verifies the published identity at handoff, and uses handle-bound cleanup;
  Windows never falls back to pathname-only deletion for these objects.
  Native Windows CI compiles the complete integration and exercises these
  filesystem guards.
- Released schema-v1 manifests are upgraded offline under the exclusive lock.
  The migration accepts the legacy collector's snapshot-symlink-as-file shape
  only after verifying every old entry and proving that every active schema-v2
  runtime blob was already checksum-bound; otherwise it fails closed and asks
  for a re-download. The schema-v2 manifest is published atomically.
- Successful manifest verification is cached in a bounded process-local
  cache. The cache key contains the canonical install directory and manifest
  SHA-256; every lookup rechecks regular files and symlink+resolved-target
  fingerprints (size and modified time plus Unix identity/change metadata or
  the full Windows volume/file identity and change time), and any change forces
  full file SHA-256 verification. Missing, absolute,
  escaping, chained, or repointed snapshot links fail closed.
- E5 runtime construction reads and re-hashes manifest-verified bytes through
  fastembed's user-defined constructor, which has no hub/download path. BGE
  builds and verifies a deterministic, private, read-only file-backed cache
  before calling fastembed; this avoids copying its 2.27 GB external
  initializer into a `Vec`, prevents the loader from following mutable source
  refs, and fails before constructor entry when an artifact is missing. ONNX
  sessions use one bounded process-wide singleflight cache keyed by preset and
  canonical install directory, and replace the session when the stable,
  layout-independent model content SHA-256 changes. Re-downloading identical
  bytes therefore does not accumulate sessions merely because cache layout or
  manifest timestamp changed, and concurrent runtime threads do not load
  duplicate sessions.
- Persisted local vector profiles are
  `<preset-model-id>@sha256:<artifact-digest>`. Coverage, backfill, and vector
  lookup already key on model id and dimensions, so new bytes automatically
  isolate old vectors without a schema migration.
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
  commit reports under `eval/provider-comparison/` with model ids, the local
  model artifact SHA-256, build profile, target OS/architecture, and thresholds.
- Default flip criteria (all required): paraphrase and provider-comparison
  slices improve, no regression beyond gate thresholds on existing slices,
  cold provider
  verification plus first profile probe within budget, and warm query-embed
  p95 within budget on a reference machine.
- The existing-slice comparison includes abstention pass rates even when the
  ordinary retrieval metrics are null. Search confidence uses a two-stage
  gate: a claim-supported grounded survivor suppresses unsupported vector-only
  tails. When a query resolves an exact stored entity, semantic fallback is
  limited to directly bound visible memories or an already-fused visible
  candidate sharing a specific entity with a direct anchor; technical/common
  tags cannot establish that bridge. Every ordinary candidate must still pass
  predicate claim coverage, while structured facts must carry an exact entity
  binding. Weak raw FTS/entity hits do not suppress unrelated zero-overlap
  paraphrase recall, and vector distance never establishes predicate truth.
- Record the flip decision and evidence links in `docs/specs/README.md` index
  entry and the epic.

GH-716 records the reference command:

```bash
REMEM_DATA_DIR=eval/provider-comparison/reference-data \
  cargo run --release --locked -- embedding download --model multilingual-e5-small
REMEM_DATA_DIR=eval/provider-comparison/reference-data \
  cargo run --release --locked -- eval-provider-comparison \
    --json-out eval/provider-comparison/report.json
```

The committed report keeps the unconditional fresh-install default unchanged
because `api` is unavailable without an explicit `--allow-api` run. It now
contains a verified local row at the default `k=5`: local reaches 1.00
paraphrase evidence recall and 0.75 provider-comparison evidence recall,
versus 0.00 for feature-hash on both slices, with 12 ms warm local
query-embedding p95. Cold provider verification plus the first profile probe
is 5431 ms, above the 1000 ms budget, so it remains a second unconditional
default-flip blocker alongside the unavailable API row. Abstention is 10/10
for both providers, and knowledge-update, temporal, and multi-hop precision
remain within budget. The local row records artifact SHA-256
`3970612d6f31b81d1dc30ddac0099da273b5753d1a07412e8390cf799e7836a6`,
derived from the logical runtime files rather than platform-specific cache
paths or symlink layout.
This evidence supports #946's quality-first conditional `Auto` activation
after an explicit download without mislabeling warm latency as cold startup.

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
- Existing feature-hash and older-artifact vectors stay valid under their model
  ids until the user backfills and prunes.

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

- GH-716 keeps automatic download and an unconditional fresh-install local
  default out of scope because the API comparison row is unavailable.
- #946 resolves `Auto` as remem-specific API key → verified installed default
  local model → feature-hash. Explicit `local` still respects its configured
  preset, while `Auto` intentionally considers only the default
  `multilingual-e5-small` profile.
