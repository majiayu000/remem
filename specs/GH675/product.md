# Product Spec

## Linked Issue

GH-675

## Implementation Issue

GH-692

## Accepted Contract

The authoritative product contract is
`docs/specs/capacity-eval-axis/PRODUCT.md`.

This SpecRail packet exists to hand the accepted #675 contract to the first
implementation issue, GH-692. It does not replace the `docs/specs/` contract.

## User Problem

Current eval gates only measure retrieval on a small fixed fixture corpus.
They do not show whether golden retrieval quality degrades as the memory store
grows with realistic non-relevant coding-project distractors.

## First Slice Goal

Ship a deterministic capacity curve command:

- synthesize scaled golden fixture corpora from the committed dataset plus
  deterministic coding-project noise;
- hold golden queries and judgments fixed across scales;
- emit JSON with seed, scale factors, corpus size, corpus hash, fused quality
  metrics, p95 latency, and largest-scale degradation versus 1x;
- prove same-seed determinism for quality metrics and corpus hashes.

## Non-Goals

- Do not implement per-channel attribution in this slice.
- Do not wire capacity budgets into `eval-gates` yet.
- Do not add #384 nightly dashboard ingestion yet.
- Do not run 50x by default in PR CI.
- Do not close GH-675 from the first-slice PR.

## Acceptance Criteria

- [ ] `remem eval-capacity` accepts dataset, seed, scales, k, JSON output, and
      JSON stdout options.
- [ ] The same seed and scales produce identical corpus hashes and quality
      metrics, excluding latency fields.
- [ ] Each reported scale includes total corpus size, noise count, fused
      R@K/nDCG/evidence recall, and p95 latency.
- [ ] The report includes largest-scale degradation from 1x for fused R@K and
      nDCG.
- [ ] Tests cover CLI parsing, deterministic synthesis, and JSON shape.

## Follow-Up

GH-675 remains open for per-channel explain attribution, eval-gates budget
enforcement, #384 dashboard ingestion, 50x nightly scheduling, and docs that
publish measured degradation after enough data is collected.
