# Summary Candidate Promotion Technical Spec

Status: Superseded reference
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #674

Superseded by `docs/specs/summary-promotion-gate/` after PR #685. Keep this
file as historical evidence for the #674 survey findings; implement from the
newer summary promotion gate contract.

## Existing Implementation Facts

- The observation-extract path inserts candidates with an
  `auto_promote_batch` and evaluates `should_auto_promote`
  (`src/memory_candidate.rs`, insertion and gate around the candidate apply
  path).
- The gate requires all of: project scope, low risk class, confidence >= 0.80,
  repo-owned route with routing confidence >= 0.80, non-empty evidence ids,
  auto-promotable memory type, no unsafe markers, and conservative
  source-observation support (`src/memory_candidate/support.rs`), with
  block reasons logged (`auto_promote_block_reason`).
- The summary path passes `None` for the batch when persisting candidates
  (`src/summarize/summary_job/persist.rs`), so the gate never runs; behavior
  is pinned by test
  `finalize_summary_creates_candidates_without_active_memories`.
- Historical gate deadlocks: #238 (type vocab mismatch), #357 (support check
  near-zero hit rate), #657 (observation type vocab mapping) — all were
  whole-class promotion stalls diagnosed late because the stall was silent.
- Doctor surfaces pending/failed extraction state (`src/doctor/database.rs`)
  but does not attribute pending_review counts to a source path.

## Design Rules

- One gate: any summary-path promotion goes through `should_auto_promote`,
  never a parallel weaker gate.
- Fail closed: missing or unloadable evidence means pending_review, not
  promotion and not an error that drops the candidate.
- Every block is attributable: the same structured block-reason logging on
  both paths.
- The pinning test changes in the same PR as the behavior, never separately.

## Proposed Design

Recommended option A (gate wiring), with option B's observability shipped in
both cases.

### A: wire the gate on the summary path

1. `persist.rs` builds a support batch from the summary's supporting
   observations/events (the same shape the observation path passes) instead of
   `None`. Where the summary pipeline lacks per-observation confidence, the
   batch marks support as summary-derived.
2. `should_auto_promote` gains a source-path parameter (enum
   `CandidateSourcePath::{ObservationExtract, SummaryJob}`) and applies
   `summary_auto_promote_min_confidence` (config; default equal to
   `AUTO_PROMOTE_MIN_CONFIDENCE`, tunable stricter) for the summary path.
3. The summary candidate builder stops hardcoding every candidate as medium
   risk. It assigns `risk_class='low'` only for source-supported,
   auto-promotable memory types that pass the same unsafe-marker and trust
   checks as observation candidates; ambiguous summary candidates remain
   `medium` and therefore stay pending_review under the existing gate.
4. Support checking: reuse `support.rs` against the summary's source
   observation texts; if the summary path cannot produce observation texts for
   a candidate, the support check fails closed (block reason
   `summary_support_unavailable`).
5. Config flag `summary_auto_promote` (default off at merge, flipped on after
   the sampling window in the rollout plan).

### B-part shipped regardless: observability

- `memory_candidates` gains a `source_path` column (migration v053 or next
  free slot; backfill existing rows via a best-effort join on summary rows,
  else `unknown`).
- Doctor: pending_review counts grouped by `source_path`, with an explicit
  label when a path has auto-promote disabled.
- `remem status`: one-line pending_review split.

### Test updates

- Replace `finalize_summary_creates_candidates_without_active_memories` with:
  - flag off: summary candidates stay pending_review (current behavior);
  - flag on + qualifying low-risk summary candidate: promotes, embedding
    upserted, operation logged;
  - flag on + current medium-risk summary candidate: stays pending_review with
    the expected risk block reason;
  - flag on + below-floor or missing-support candidate: pending_review with
    the expected block reason.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| A1 same gate, stricter floor | should_auto_promote | boundary tests at both floors and summary risk classes |
| A2 block-reason parity | persist.rs + gate | test asserts structured reason on summary block |
| A3 evidence required | support batch construction | test: no evidence -> pending, reason logged |
| B1/B2 observability | schema + doctor | test: counts grouped by source_path; label rendered |
| Common 4 decision recorded | this spec | spec update in the implementation PR |
| Common 5 sampling | real sessions | evidence comment on #674 |

## Data Flow

summary job -> candidate rows (now with source_path) -> gate (flag-gated on
summary path) -> promoted memory + embedding + operation log, or
pending_review + block reason -> doctor/status attribution -> review inbox.

## Alternatives Considered

- Separate, weaker summary gate: rejected; parallel gates are how #238-style
  vocabulary drift happens.
- Auto-approving summary candidates above a very high confidence without
  support checks: rejected; support checking is the anti-hallucination
  backstop and summaries are the most hallucination-prone extraction source.
- Doing only B (documentation + observability): kept as the fallback decision;
  the spec records the choice either way. The cost of A is modest because the
  gate machinery already exists.

## Risks

- Security: none new; the gate's unsafe-marker and (with #672) trust checks
  apply unchanged.
- Compatibility: additive column with backfill; flag-off preserves current
  behavior byte-for-byte.
- Performance: gate evaluation per summary candidate is in the background
  worker path, not hooks.
- Maintenance: one more config knob; documented next to the existing promote
  floor.

## Test Plan

- [ ] Unit tests: gate source-path parameter, summary floor boundaries,
      fail-closed support behavior.
- [ ] Integration test: full summary job on a fixture session with the flag
      on; assert promotion set and block set.
- [ ] Manual verification: one real Claude Code session and one Codex session
      with the flag on; `remem doctor` shows the split and any promotions;
      results posted to #674.

## Rollback Plan

Turn `summary_auto_promote` off (restores exact current behavior). The
`source_path` column and doctor split remain — they are pure observability
and are the fix for the silent-stall class regardless of the gate decision.
