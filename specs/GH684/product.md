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
- [x] Field-level equivalence fixtures compare legacy `finalize_summarize`
      output with current `persist_session_rollup` output before Summary job
      retirement.
- [x] `pending_observations` emptiness is confirmed on real databases beyond
      the primary dogfood store, or stragglers are migrated explicitly.
- [x] In-flight `JobType::Summary` upgrade handling is implemented and tested:
      non-terminal legacy Summary jobs are rejected as permanent failures by
      migration v064, and already-claimed Summary jobs are rejected by the
      worker before the retired AI/finalize path can run. Stop hooks no longer
      enqueue new Summary jobs, capture-ledger failures spill instead of
      falling back to the retired writer, same-host/project/session stale
      spills are skipped after the current stop payload succeeds while other
      projects still replay, replayed Stop captures are idempotent after later
      replay-step failures, duplicate replay captures with the same fixed event
      ID do not revive completed rollup tasks, replay capture-ledger failures
      preserve one active spill row, retryable failed Summary rows are frozen
      before failure maintenance can reopen them, v064 upgrade rejection rows
      are not actionable doctor/status failures, worker-side post-retirement
      Summary rejections stay visible, the Stop hook keeps only immediately
      available citation/failure side effects, and transcript-only signals run
      after worker-side raw archive ingest bounded by the captured Stop byte
      length. A coalesced rollup drains every covered Stop payload while
      deduplicating repeated transcript paths, and summary-derived candidates
      cite only captured events inside the persisted rollup range. Persisted
      rollups re-home summary-derived candidates, workstream upsert, and
      native-memory sync. The #792 slice captures only command-result-proven
      commits, preserves typed evidence through spill/replay, and links every
      commit from the exact claimed ObservationExtract or SessionRollup range;
      evidence recovered after a completed cursor uses bounded link-only work
      and never replays SessionRollup AI or side effects. Transcript evidence
      keeps earlier proven calls when another shell call is ambiguous,
      malformed, or its candidate metadata cannot be resolved, anchors relative
      workdirs to the Stop cwd, recognizes only an exact trailing
      `git status --short`, and makes same-identity spill evidence deterministic
      and link-only. Legacy spill rows without an `event_id` receive stable,
      occurrence-distinct identities so byte-identical rows do not collapse
      and failed replay
      preserves the assigned identity; platforms without PID liveness restore
      orphan claims after the minimum-age gate. Its fail-closed command grammar
      accepts ordinary spaced/equal `--fixup` commits but rejects editor-opening
      amend/reword fixups, environment prefixes, arbitrary Git configuration,
      help/viewer/editor output, dry runs, interactive add modes, shell
      expansion, redirection, globbing, process substitution, or unquoted shell
      comments. Quiet commits remain ordinary captured events but cannot create
      links because they suppress Git's own commit summary. Success requires a
      zero exit status or Claude's
      explicitly named success-only `PostToolUse` event; an explicit failure
      event overrides contradictory fields. Unknown/failure events and
      status-like text inside Codex command output cannot prove success.
      The #794 follow-up also feeds user/assistant text from each selected Stop
      transcript snapshot into the rollup prompt through the captured
      `transcript_byte_len`, deduplicates repeated paths at the widest covered
      boundary, and omits exact text already represented by captured events.
      One count- and byte-bounded, redacted evidence slice is shared by the
      prompt and candidate support path, then persisted with an exact-range raw
      archive completion checkpoint so retries do not depend on a removed
      transcript file. Per-Stop assistant-message hashes and structured
      citation facts are persisted outside the lossy prompt slice so long-tail
      or earlier-Stop citations survive prompt eviction and source deletion;
      repeated paths retain facts for each bounded Stop. Early v066 JSON reuses
      its original bounded hash during retry. A legacy Stop without a captured
      boundary uses only
      captured conversational events; without that fallback it fails
      permanently before AI. Missing, malformed, or unusable required bounded
      snapshots still block metadata-only summary persistence.
      Compress/Dream follow-up jobs are enqueued only after the rollup is
      persisted, old-version daemon heartbeats and legacy singleton locks do
      not suppress the current Stop fallback worker, a current once-launch
      heartbeat prevents overlapping fallback workers, workers run
      SessionRollup extraction before Compress/Dream jobs, and terminal Summary
      history plus non-summary jobs are preserved. The #792 observed-commit,
      #794 bounded prompt-evidence, #795 native-memory failure-isolation, and
      #796 exact-range follow-up scheduling slices are implemented. Automatic
      mirror failures stay error-visible without blocking durable follow-ups;
      retries preserve terminal Compress/Dream history while new ranges remain
      eligible for their own scheduling decision. Pre-v068 exact ranges retain
      an error-visible `legacy_unknown` decision instead of inferred replacement
      jobs, including ranges inserted late by an already-running old worker;
      v068 requeues those processing leases. New decisions retain exact
      Compress/Dream job attribution and
      distinguish enqueued, inflight-coalesced, and cooldown-suppressed Dream
      outcomes.
- [x] MCP/docs wording stops calling live `observations` legacy.
- [x] Doctor reports legacy row counts and errors when frozen surfaces receive
      writes.

## Edge Cases

- Stop-hook Summary retirement must preserve non-summary side effects such as
  Compress enqueueing, Dream enqueueing, raw archive ingest, citation handling,
  failure lessons, candidate finalization, and native memory sync.
- Stop payload redaction must preserve path fields needed by worker-side raw
  ingest while continuing to redact sensitive keys.
- Multiple Stop captures coalesced into one rollup must not lose earlier
  transcript or hook-fallback messages, and later captures outside the claimed
  range must not become evidence for the earlier rollup's candidates.
- Transcript prompt evidence must remain bounded, deterministic, redacted, and
  XML-escaped; every supplemental message stays anchored to a Stop event inside
  the claimed range.
- Drop migrations must refuse to run while unmigrated valuable rows remain.
- Current reader surfaces must not lose context, timeline, or `why` behavior.

## Rollout Notes

Each phase is independently reviewable: visibility first, equivalence fixtures
before writer retirement. The removal window is now active: remem 0.6.0 shipped
the doctor announcement in source, and the superseding remem 0.6.1 GitHub
release published the migration commands and the no-earlier-than-0.7.0 removal
notice after v0.6.0's Create Release step failed. No guarded drop may ship
before remem 0.7.0.
