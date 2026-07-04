# Tech Spec

## Linked Issue

GH-682

## Implementation Issues

- GH-714: Phase 1 provider contract and degraded-state visibility
- GH-715: Phase 2 local semantic ONNX model, same-model vectors, and backfill
- GH-716: Phase 3 provider comparison eval gate and default-flip evidence
- GH-717: Phase 4 downstream semantic dedup and preference adoption

## Accepted Contract

The authoritative technical contract is
`docs/specs/local-semantic-embedding/TECH.md`.

This SpecRail packet splits the accepted epic into implementation phases. It
does not replace the `docs/specs/` contract.

## Codebase Context

| Area | Files | Current behavior | Phase scope |
| --- | --- | --- | --- |
| Provider config | `src/retrieval/embedding.rs`, install/config writers | `Auto` resolves to API when a key exists, otherwise `FeatureHash` until GH-716 eval evidence; `local` and `feature-hash` are separate provider states. | GH-714 separates provider contract states and preserves env override compatibility. |
| Status/doctor | `src/cli/actions/status.rs`, doctor modules, shared stats queries | Embedding provider readiness and active-model vector coverage are not a first-class visible contract. | GH-714 exposes configured/active provider, degraded/disabled state, model id, and coverage. |
| Vector persistence | migrations, `src/retrieval/vector.rs`, memory write/delete paths | `memory_embeddings` uses `(memory_id, model, dimensions)` so multiple active/fallback profiles can coexist. | GH-715 keeps upsert/delete behavior model-aware. |
| Local semantic runtime | new embedding runtime/download modules, CLI dispatch | `local` uses fastembed-backed ONNX presets with verified manifests; missing models report unavailable instead of aliasing feature-hash. | GH-715 adds download/status, manifest checksum plus hf-hub LFS source-sha verification, model-dir resolution, hook-safe readiness, and query/write integration. |
| Backfill | new embedding backfill command/action | Provider switching has active-model coverage reporting and explicit backfill/prune workflow. | GH-715 adds idempotent backfill and explicit prune gating. |
| Eval evidence | `eval/`, `src/eval/`, CLI eval commands | Golden fixtures include paraphrase notes, but no committed provider comparison gate for local semantic default decisions. | GH-716 adds provider comparison reports and default-flip criteria. |
| Downstream consumers | `src/memory/dedup/funnel.rs`, `src/memory/semantic_dedup.rs`, `src/memory/store/write.rs`, `src/memory/operation.rs`, preference consolidation | Semantic dedup/preference paths use feature-hash cosine fallback or have an open vector-stage TODO. | GH-717 adopts active semantic model semantics and recalibrates thresholds. |

## Proposed Design

Implement the accepted contract in four PR-sized phases:

1. GH-714 adds the provider-resolution and visibility contract without
   downloading or executing a new local model.
2. GH-715 adds the local semantic runtime, model inventory commands,
   multi-model vector storage, same-model-id scoring, and backfill.
3. GH-716 extends eval fixtures and commits provider comparison evidence before
   any default provider change.
4. GH-717 moves downstream dedup/preference consolidation to the active
   semantic space after the eval gate proves model quality and thresholds.

## Product-To-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| Provider state is explainable | Config resolver, status JSON, doctor findings | Config resolution tests plus status/doctor tests in GH-714 |
| Fallback is visible | Resolver diagnostics and logging | Degraded-provider test asserts status/doctor and error-level log in GH-714 |
| `off` disables vector behavior | Search/write/backfill provider checks | Off-provider tests in GH-714 |
| Weights are not bundled | Download command and model-dir resolver | Download/status tests and packaging review in GH-715 |
| Same-model cosine only | Vector query filters and persistence schema | Mixed-model store regression tests in GH-715 |
| Backfill reports coverage | Backfill command/action | Idempotency, coverage, and prune-gating tests in GH-715 |
| Default flip is evidence-bound | Eval provider comparison artifacts | Eval-gate tests and committed reports in GH-716 |
| Downstream consumers use active semantics | Dedup and preference code paths | Dedup/preference regression fixtures in GH-717 |

## Data Flow

Configured provider and env overrides resolve to an active embedding profile.
Write paths produce vectors only for the active non-off provider and persist
them with model id and dimensions. Query paths embed the query with the active
profile, score only vectors with the same model id, and surface degraded
metadata when fallback is active. Backfill walks the searchable memory set,
fills missing vectors for the active model, reports coverage, and prunes
other-model vectors only with explicit user intent after full coverage.

## Alternatives Considered

- Bundle model weights in the binary: rejected by #682 and the accepted spec
  because release assets would become large and platform-sensitive.
- Keep `local` as feature-hash forever: rejected because it keeps the highest
  retrieval channel pseudo-semantic for default installs.
- Flip the default before eval evidence: rejected by the epic; default changes
  require committed comparison reports.
- Merge all phases in one PR: rejected because provider config, runtime/model
  download, eval gates, and downstream dedup have different review risks.

## Risks

- Security: model download must use explicit URLs, checksum verification, and
  no credential logging.
- Compatibility: existing env variables and feature-hash vectors must continue
  to work.
- Performance: query embedding and brute-force cosine must stay within search
  latency budgets; hooks must not block on download.
- Maintenance: ONNX runtime dependency choice affects release targets and
  binary size.

## Test Plan

- [ ] GH-714: config resolution, degraded fallback, status JSON, doctor human
      and JSON tests.
- [ ] GH-715: migration, same-model guard, download/status, hook-safe missing
      model, backfill idempotency, and prune-gating tests.
- [ ] GH-716: provider comparison eval fixtures and reports under `eval/`,
      plus eval command tests.
- [ ] GH-717: dedup, semantic dedup, and preference consolidation regression
      tests.
- [ ] Each phase: `cargo fmt --check`, `cargo check --message-format=short`,
      focused tests, and `cargo test` before merge readiness.

## Rollback Plan

Each phase must be independently revertible. Provider config changes retain
feature-hash and `off` escape hatches. Local runtime rollout can be disabled by
selecting `feature-hash` or `off`. Default-provider changes are blocked until
GH-716 records evidence and can be reverted by restoring the previous default
provider while preserving stored model-id vectors.
