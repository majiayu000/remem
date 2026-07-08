# Product Spec

## Linked Issue

GH-684

## Accepted Contract

The authoritative product contract is
`docs/specs/legacy-observation-retirement/PRODUCT.md`.

This SpecRail packet hands the existing #684 contract to workflow tracking. It
does not replace the `docs/specs/` contract and does not approve runtime
implementation by itself.

## User Problem

remem still carries multiple historical observation-era surfaces. The verified
inventory reframed the debt: `pending_observations` is a dead queue,
`observations` is a live current intermediate whose legacy wording is fixed by
GH684-T8, and `session_summaries` is load-bearing but dual-written by both the
current SessionRollup path and the legacy Summary job chain.

## Goals

- Record one disposition per legacy or suspected-legacy surface.
- Retire dead or duplicated write paths without losing user-visible history.
- Reclassify current surfaces accurately so docs and MCP descriptions do not
  mislead users or future contributors.
- Make legacy state observable in doctor before any freeze or removal.

## Non-Goals

- No second rewrite of the capture pipeline.
- No default-path behavior change without equivalence fixtures.
- No silent table drops or hidden data loss.
- No removal of `observations`, `observations_fts`, or `session_summaries`
  when the verified disposition says they are current or load-bearing.

## Current Verified Dispositions

- `pending_observations`: retire after confirming real databases have no
  unmigrated rows and keeping `remem pending migrate-legacy` as an escape
  hatch.
- `observations`: reclassify current; keep GH684-T8 wording from regressing
  rather than retire the table.
- `observations_fts`: current, trigger-maintained, follows `observations`.
- `session_summaries` table: keep; it remains load-bearing.
- Legacy Summary job chain: retire only after field-level output equivalence
  and Stop-hook side effects are preserved elsewhere.

## Acceptance Criteria

- [ ] TECH inventory stays current with every production writer and reader.
- [ ] Field-level equivalence fixtures compare legacy `finalize_summarize`
      output with current `persist_session_rollup` output before Summary job
      retirement.
- [ ] `pending_observations` emptiness is confirmed on real databases beyond
      the primary dogfood store, or stragglers are migrated explicitly.
- [x] In-flight `JobType::Summary` upgrade handling is decided and tested:
      non-terminal legacy Summary jobs are rejected as permanent failures by
      migration v064, and already-claimed Summary jobs are rejected by the
      worker before the retired AI/finalize path can run. Stop hooks no longer
      enqueue new Summary jobs, capture-ledger failures spill instead of
      falling back to the retired writer, and terminal Summary history plus
      non-summary jobs are preserved.
- [x] MCP/docs wording stops calling live `observations` legacy.
- [ ] Doctor reports legacy row counts and errors when frozen surfaces receive
      writes.

## Edge Cases

- Stop-hook Summary retirement must preserve non-summary side effects such as
  Compress enqueueing, Dream enqueueing, raw archive ingest, citation handling,
  failure lessons, candidate finalization, and native memory sync.
- Drop migrations must refuse to run while unmigrated valuable rows remain.
- Current reader surfaces must not lose context, timeline, or `why` behavior.

## Rollout Notes

Each phase is independently reviewable: visibility first, equivalence fixtures
before writer retirement, deprecation window before any guarded drop.
