# Tech Spec

## Linked Issue

GH-675

## Implementation Issue

GH-692

## Accepted Contract

The authoritative technical contract is
`docs/specs/capacity-eval-axis/TECH.md`.

This SpecRail packet narrows implementation to the first deterministic
capacity-curve slice.

## Codebase Context

| Area | Files | Current behavior | First-slice change |
| --- | --- | --- | --- |
| Golden eval | `src/eval/golden/`, `eval/golden.json` | Evaluates a fixed fixture corpus. | Reuse the same loader and search path while adding deterministic scaled fixture datasets. |
| Eval CLI | `src/cli/types.rs`, `src/cli/eval_types.rs`, `src/cli/actions/eval.rs`, `src/cli/dispatch.rs` | Has `eval`, `eval-gates`, `eval-graph-decision`, and related commands. | Add `eval-capacity` args and action. |
| Metrics | `src/eval/golden/types.rs`, `src/eval/metrics/` | Golden report already computes R@K, nDCG, evidence recall, and latency. | Summarize fused quality metrics per scale and compute degradation from 1x. |
| Synthesis | new `src/eval/capacity.rs` or `src/eval/capacity/` | No capacity corpus generator. | Add deterministic noise generation from fixed slot pools and seed/scale values. |
| Tests | `src/eval/*/tests.rs`, `src/cli/tests_eval.rs` | No capacity tests. | Add deterministic synthesis, JSON shape, and CLI parsing tests. |

## Design Rules

- Determinism must not use wall-clock, random OS state, or unordered maps in
  corpus generation or quality metric output.
- Golden queries and evidence refs are reused unchanged; only non-relevant
  noise memories are appended.
- Noise topic keys must be unique and must not collide with existing fixture
  topic keys.
- Latency is reported but excluded from same-seed determinism assertions.
- This slice reports fused retrieval only. Channel attribution is follow-up
  work, not silently faked.

## Proposed Design

Create `src/eval/capacity.rs` with:

- `CapacityEvalOptions { dataset_path, seed, scales, k }`.
- `run_capacity_eval(options) -> CapacityEvalReport`.
- `synthesize_capacity_dataset(dataset, seed, scale) -> ScaledDataset`.
- stable corpus hashing over the synthesized corpus fields relevant to search.

Noise generation:

- total corpus size for scale `N` is `base_corpus_len * N`;
- `scale=1` adds no noise;
- larger scales append deterministic memories using fixed pools for file paths,
  crate names, error strings, command lines, and owners;
- memory type rotates over `decision`, `bugfix`, `discovery`, and `lesson`;
- topic keys include seed, scale, and index to avoid collisions.

Report shape:

```json
{
  "version": "2026-07-02",
  "dataset_path": "eval/golden.json",
  "seed": 42,
  "k": 5,
  "scales": [
    {
      "scale": 1,
      "corpus_size": 50,
      "noise_count": 0,
      "corpus_hash": "...",
      "fused": {
        "recall_at_k": 0.95,
        "ndcg_at_10": 0.93,
        "evidence_recall_at_k": 0.95,
        "p95_latency_ms": 1.2
      }
    }
  ],
  "degradation": {
    "largest_scale": 10,
    "fused_recall_at_k_loss": 0.02,
    "fused_ndcg_at_10_loss": 0.01
  }
}
```

## Verification Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| CLI emits capacity curve | CLI dispatch + action | CLI parse test and command smoke |
| Same seed deterministic | capacity generator + report | unit test strips latency and compares reports |
| Fixed judgments | dataset synthesis | test confirms query ids/evidence refs unchanged |
| Scale metadata | report types | JSON shape test |
| Degradation | report calculation | unit test with 1x and largest scale |

## Rollback

Remove the additive `eval-capacity` command and `src/eval/capacity` module. No
runtime data, schema, hook, or API behavior changes.
