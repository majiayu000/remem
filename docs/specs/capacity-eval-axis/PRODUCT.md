# Capacity Eval Axis Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #675
- Related: #384 (nightly eval dashboard, regression budget), #328 (retriever vs answer metrics, closed)

## Problem

Every current eval gate runs against small fixture databases. Nothing measures
whether retrieval quality degrades as the store grows — the axis where
production memory systems are known to collapse (Mem0's own published numbers
fall from 92.5 at benchmark scale to 64.1 at 1M tokens and 48.6 at 10M;
MemBench, arXiv 2506.21605, names this the capacity axis).

remem's promise is long-lived per-project memory, so its databases only grow.
A six-month-old project runs a different retrieval problem than the fixture
suite tests, and today a capacity regression would ship invisibly.

## Goals

- Measure retrieval quality and latency as a degradation curve over store
  size, deterministically and in CI.
- Attribute degradation per retrieval channel so fixes target the right
  channel.
- Give #384's nightly dashboard a capacity series with a regression budget.

## Non-Goals

- Real user data in fixtures.
- Optimization work: this spec is measurement only; fixes are follow-up issues
  justified by the curve.
- Benchmarking extraction or injection at scale (retrieval only).

## Behavior Invariants

1. The capacity harness synthesizes grown databases at declared scale factors
   (default 1x / 10x / 50x of the golden corpus) from templates plus
   deterministic noise memories; the same seed always produces byte-identical
   corpora and identical metrics.
2. Golden queries and their relevance judgments are held fixed across scales;
   only distractor volume grows.
3. The report includes, per scale and per channel (FTS, vector, entity,
   temporal, fused): R@5, nDCG, and p95 search latency.
4. A regression budget compares the largest scale against 1x; a breach fails
   the eval gate with the offending channel named.
5. Noise memories are realistic for coding projects (plausible file paths,
   crate names, error strings) so lexical channels are actually stressed, and
   are labeled as noise in the fixture so judgments stay unambiguous.

## Acceptance Criteria

- [ ] `remem eval` (or a dedicated subcommand) emits the capacity curve as
      JSON with corpus seed, scale factors, and per-channel metrics.
- [ ] Two runs with the same seed produce identical JSON (modulo latency
      fields, which are reported but excluded from determinism assertions).
- [ ] The eval gate enforces the regression budget; a synthetic injected
      regression (test-only) demonstrably fails it.
- [ ] Nightly dashboard (#384) ingests the series; eval docs state the
      measured degradation at each scale.

## Edge Cases

- Vector channel with the default feature-hash embedding: the curve is still
  reported (it measures the shipped default), with the embedding provider
  recorded in the JSON so provider comparisons are possible later.
- Latency on shared CI runners: latency is trend data, not a hard gate, to
  avoid flaky failures; quality metrics are the gated values.
- Scale factors that exceed practical CI time: 50x runs nightly only; PR CI
  runs 1x/10x.

## Rollout Notes

Eval-only change; no runtime behavior or schema impact. Land the harness and
report first, observe one week of nightly data, then set the budget from
observed variance rather than guessing.
