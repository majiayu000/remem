# Product Spec

## Linked Issue

GH-716

## User Problem

remem now has a real local semantic embedding runtime, but the project still
needs committed evidence before changing any default provider. Without a
provider-comparison report, users cannot tell whether local semantic embeddings
actually improve paraphrase recall enough to justify a default flip, and
maintainers cannot distinguish "model not installed", "API unavailable", and
"provider quality regressed" outcomes.

## Goals

- Add explicit provider-comparison eval evidence for `feature-hash`, `local`,
  and `api` embedding providers before any default provider change.
- Extend golden fixtures with English and CJK paraphrase/synonym cases that
  exercise the semantic gap between feature-hash and learned embeddings.
- Make the default-flip decision observable in committed docs/spec metadata.
- Keep unavailable providers honest: missing local model files or missing API
  credentials must be reported as unavailable, not treated as passes.

## Non-Goals

- Do not change the default embedding provider unless the committed evidence
  satisfies the flip criteria.
- Do not require CI or ordinary contributors to download model weights or call
  a paid embedding API.
- Do not move dedup or preference consolidation onto active semantic embeddings;
  GH-717 owns that follow-up.
- Do not tune fusion weights in this issue.

## Behavior Invariants

1. Provider comparison reports include one row for each required provider:
   `feature-hash`, `local`, and `api`.
2. Each provider row records provider id, model id when known, availability,
   skip/unavailable reason when not runnable, retrieval metrics for runnable
   providers, and query embedding latency summary.
3. Golden eval fixtures include English and CJK paraphrase/synonym cases with a
   stable slice label so regressions are visible independently of the full
   golden aggregate.
4. Default-flip criteria are explicit: paraphrase slice improves over
   feature-hash, existing golden slices remain within gate thresholds, and p95
   query embedding latency stays within the documented search budget.
5. If local or API comparison cannot run because the model or credentials are
   unavailable, the report records a non-flip decision with the blocker.
6. The docs/spec index links to the committed comparison evidence and states
   whether the default changed or stayed unchanged.

## Acceptance Criteria

- [x] A provider-comparison command or eval mode emits JSON that covers
      `feature-hash`, `local`, and `api` provider states.
- [x] `eval/golden.json` contains English and CJK paraphrase/synonym fixtures
      tagged with a provider-comparison slice.
- [x] A committed report under `eval/` records the current provider comparison
      and default-flip decision.
- [x] `docs/specs/README.md` and the GH-682 evidence trail reference the report
      and decision.
- [x] Verification includes provider-comparison focused tests, `cargo test
      eval`, extraction baseline, eval gates, fmt, check, and GH682 workflow
      check.

## Edge Cases

- Local model unavailable: report `available=false` with a missing-model reason
  and do not download automatically.
- API key missing or endpoint unavailable: report `available=false` or
  `degraded=false` for API evidence; do not fall back silently in the API row.
- Existing feature-hash behavior remains the baseline and must always be
  runnable in CI.
- Provider comparison should be deterministic for the checked-in fixture set
  except for measured latency fields.

## Rollout Notes

This issue may keep the default provider unchanged. The decision is successful
when it is explicit, evidence-linked, and honest about unavailable providers.
Use `Closes #716` for the implementation PR and `Refs #682` for the epic.
