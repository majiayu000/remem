# Product Spec

## Linked Issue

GH-674

## Accepted Contract

The authoritative product contract is
`docs/specs/summary-promotion-gate/PRODUCT.md`.

This SpecRail packet hands the accepted #674 contract to implementation
issues. Phase 1 was GH-690. The current slice is Phase 2, GH-696. It does not
replace the `docs/specs/` contract.

## User Problem

Summary-derived candidates are stored as `pending_review` with no source-path
split and no way to tell whether a summary candidate would have promoted if the
summary path had a safe gate. This hides a whole stalled input class from
doctor/status output and blocks the production evidence chain for #381/#383.

## Phase 1 Goal

Ship observability plus a shadow summary gate:

- new candidate rows identify their source path;
- legacy ambiguous rows remain explicitly unattributed;
- summary-derived decisions and discoveries are evaluated by the proposed gate
  in shadow mode;
- would-promote candidates remain pending until Phase 2, but are counted and
  attributed;
- unsupported summary candidates get source-support block reasons instead of a
  masked `risk_class_not_low` reason.

## Phase 2 Goal

Enable the summary gate for supported factual summaries:

- default `promotion.summary_gate_mode` becomes `enforce`;
- qualifying summary-derived `decision` and `discovery` candidates promote
  automatically when the source-support and route gates pass;
- `shadow` and `off` modes remain available as rollback controls;
- `lesson` and `preference` summary candidates remain review-gated;
- unsupported summary candidates continue to record source-support block
  reasons and stay pending.

## Non-Goals

- Do not bulk approve or replay the existing `pending_review` backlog.
- Do not relax the observation-path gate or its thresholds.
- Do not add LLM calls to promotion.
- Do not close GH-674 until Phase 2 sampling evidence is recorded on the
  tracking issue.

## Acceptance Criteria

- [ ] Existing ambiguous candidates stay `source_kind = unattributed` unless a
      deterministic backfill proves their source.
- [ ] New observation candidates persist as `observation`; new summary
      candidates persist as `summary`.
- [ ] A supported summary-derived decision/discovery candidate in shadow mode
      remains `pending_review`, records `summary_gate_shadow`, and increments
      would-promote telemetry.
- [ ] An unsupported summary candidate records
      `summary_source_support_unavailable` or
      `summary_source_support_failed`.
- [ ] Doctor/status promotion output is split by source kind and includes the
      summary shadow count.
- [ ] In enforce mode, a supported summary-derived decision/discovery candidate
      auto-promotes with no `auto_promote_block_reason`.
- [ ] In shadow mode, the same supported candidate stays pending with
      `summary_gate_shadow`.
- [ ] In off mode, summary candidates stay pending with `summary_gate_off`.
- [ ] Summary-derived lessons and preferences stay review-gated.

## Follow-Up

GH-674 remains open until Phase 2 threshold selection, enforcement, and
real-session sampling evidence are all recorded.
