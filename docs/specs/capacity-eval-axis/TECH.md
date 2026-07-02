# Capacity Eval Axis Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #675

## Existing Implementation Facts

- Golden corpus: `eval/golden.json` (50 fixture queries incl. multi_hop) with
  harness under `src/eval/golden/`.
- Metrics: `precision_at_k` / `recall_at_k`
  (`src/eval/metrics/retrieval.rs`), nDCG (`src/eval/metrics/ranking.rs`).
- Retrieval channels and fusion: `src/retrieval/search/memory/text.rs` with
  per-channel contributions already exposed via `--explain`
  (`build_explain`).
- Eval CLI family: `eval`, `eval-local`, `eval-gates`, `eval-weight-grid`
  (`src/cli/dispatch.rs`); `eval-gates` emits JSON consumed by CI.
- Latency benchmarks exist (`tests/search_latency_benchmark.rs`).
- Determinism constraint: existing golden eval is seed-free because the corpus
  is static; capacity needs seeded synthesis.

## Design Rules

- Deterministic synthesis: a fixed seed and template set fully determine the
  corpus; no wall-clock, no RNG without the seed.
- Judgments never change with scale: noise memories are constructed to be
  non-relevant by design (disjoint answer keys), not judged after the fact.
- Per-channel attribution reuses the existing explain machinery rather than a
  parallel scoring path.
- Latency is reported, quality is gated.

## Proposed Design

### Corpus synthesis

New module `src/eval/capacity/`:

1. Templates: a curated set of noise-memory templates per memory type
   (decision, bugfix, discovery, lesson) with slot fillers (file paths, crate
   names, error signatures, command lines) drawn from deterministic pools.
2. Generator: `synthesize(seed, scale)` expands the golden corpus DB by
   inserting `(scale - 1) * base_count` noise memories, round-robin over
   types, with embeddings upserted through the normal write path so the vector
   channel is honestly stressed. `scale=1` is the golden corpus with no
   distractors, `scale=10` is ten times the base corpus size, and reported
   labels always mean total corpus scale rather than distractor multiple.
3. Guardrail: a build-time check asserts no noise memory shares a topic_key,
   state_key, or answer-bearing token set with any golden judgment target
   (disjointness is verified, not assumed).

### Run and report

`remem eval-capacity --seed <n> --scales 1,10,50 --json-out <path>`:

- For each scale: build DB (cached by (seed, scale) content hash in the eval
  workspace), run all golden queries through the standard search entry point,
  collect fused and per-channel rankings via the explain output, compute
  R@5 / nDCG per channel and fused, and measure p95 latency over the query
  set.
- JSON output: `{seed, scales, embedding_provider, per_scale: {scale, channels:
  {…}, fused: {…}, p95_latency_ms}}` plus `degradation`: positive quality loss,
  computed as 1x fused metrics minus largest-scale fused metrics.

### Gating

- `eval-gates` gains a capacity slice: fail when positive quality loss
  `degradation.fused.r_at_5 > budget` (budget in the gates config; initial
  value set after one week of nightly data per the rollout plan).
- PR CI runs scales 1,10; nightly runs 1,10,50 and publishes the JSON trend
  artifact for the #384 dashboard.
- Latency fields excluded from determinism assertions and from gating.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 deterministic synthesis | eval/capacity generator | test: two runs same seed -> identical corpus hash and metrics JSON |
| P2 fixed judgments | disjointness guardrail | test: guardrail rejects a crafted colliding template |
| P3 per-channel report | explain integration | test: JSON contains all active channels at each scale |
| P4 budget gate | eval-gates slice | test: injected synthetic regression fails the gate |
| P5 realistic noise | template pools | review + test that pools cover paths/crates/errors |

## Data Flow

seed + templates -> synthesized DBs (cached) -> standard retrieval entry point
-> explain-derived per-channel rankings -> metrics JSON -> eval-gates budget
check + nightly trend artifact -> #384 dashboard.

## Alternatives Considered

- Growing the corpus by duplicating golden memories with perturbations:
  rejected; near-duplicates of judgment targets corrupt judgments and test
  dedup rather than capacity.
- Replaying anonymized real-project DBs: rejected for CI (non-deterministic,
  privacy review burden); may become a manual companion study.
- Gating latency: rejected; shared-runner variance would make the gate flaky
  (W-16-style fresh evidence still available via the reported trend).

## Risks

- Security: none; fixtures are synthetic.
- Compatibility: none; eval-only surface.
- Performance: 50x corpus build time in nightly; mitigated by (seed, scale)
  caching.
- Maintenance: template pools need occasional refresh as memory types evolve;
  the disjointness guardrail keeps refreshes safe.

## Test Plan

- [ ] Unit tests: generator determinism, disjointness guardrail, JSON shape.
- [ ] Integration test: full 1x/10x run in test profile with budget check on a
      known-good and a synthetically regressed configuration.
- [ ] Manual verification: one nightly run; inspect the degradation curve and
      record the initial budget in the gates config.

## Rollback Plan

Remove the capacity slice from eval-gates config; the subcommand and module
are inert without the gate. No runtime or schema surface to revert.
