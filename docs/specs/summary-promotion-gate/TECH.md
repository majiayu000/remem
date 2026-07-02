# Summary Promotion Gate Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #674

## Existing Implementation Facts

Verified against `main` (3c63b99), 2026-07-02:

- Summary call chain: `src/summarize/summary_job/persist.rs` calls
  `crate::memory::promote_summary_to_memory_candidates`
  (`src/memory/promote/summary.rs`), which calls
  `persist_summary_candidates` (`src/memory_candidate.rs`), which calls
  `persist_candidate_rows(..., None)` — the `auto_promote_batch` argument is
  always `None` on this path.
- Auto-promote fires only inside
  `auto_promote_batch.is_some_and(|batch| should_auto_promote(...))`, so the
  summary path structurally cannot promote.
- `SUMMARY_CANDIDATE_RISK: &str = "medium"` and
  `SUMMARY_CANDIDATE_CONFIDENCE: f64 = 0.74` are hardcoded in
  `src/memory/promote/summary.rs`; lessons get 0.70–0.85 via
  `lesson_confidence`.
- `should_auto_promote` requires `risk_class == "low"` and
  `confidence >= AUTO_PROMOTE_MIN_CONFIDENCE` (0.80). The block-reason
  function checks risk before batch, so summary candidates always record
  `risk_class_not_low`; `missing_source_observation_batch` is unreachable in
  practice on this path.
- `MemoryType::auto_promote` allows architecture/bugfix/decision/discovery
  only. The summary path emits decision/discovery/lesson/preference.
- Stats already group candidates by
  `review_status + auto_promote_block_reason`
  (`src/db/query/stats.rs`), but nothing records which pipeline produced a
  candidate, so no per-path split is possible today.
- Behavior is pinned by
  `finalize_summary_creates_candidates_without_active_memories`
  (`src/summarize/summary_job/persist.rs` tests).

## Design

### 1. Source-path column

Migration adds `source_kind TEXT NOT NULL DEFAULT 'unattributed'` to
`memory_candidates` (values: `observation`, `summary`, `unattributed`;
extendable). Existing rows remain `unattributed` unless a deterministic
backfill can prove their source. A row may be backfilled to `observation` only
when the persisted candidate is tied to an observation extraction batch or
observation-derived evidence. A row may be backfilled to `summary` only when
the persisted candidate can be traced to summary-job provenance. Ambiguous
pre-migration rows stay `unattributed` so doctor/status do not hide the
historical summary backlog inside the observation bucket.

`persist_candidate_rows` takes the source kind from its caller:
`persist_candidates` writes `observation`, `persist_summary_candidates`
writes `summary`.

### 2. Summary promotion gate

A summary candidate has no observation batch, but summary support is not
inherent: the summary fields are produced by `call_summary_ai`, while current
evidence ids point at the Stop/session_stop hook event. A non-empty Stop event
id proves where the summary was captured, not that the candidate text is
grounded. The summary path therefore gets its own gate, and that gate must
expand each candidate to its underlying transcript, captured events, or
observation text before it can promote:

`should_auto_promote_summary(candidate, route, evidence_json)` requires:

- `scope == "project"`
- memory type in the summary allowlist: `decision`, `discovery`
  (`lesson`/`preference` stay review-gated; `MemoryType::auto_promote`
  vocabulary is unchanged)
- `confidence >= SUMMARY_AUTO_PROMOTE_MIN_CONFIDENCE` (initial Phase 1 floor
  0.70 so today's decision/discovery summaries at 0.74 exercise the shadow
  path; Phase 2 floor is recorded from telemetry before enforcement)
- `route.is_repo_owned()` and
  `route.routing_confidence >= AUTO_PROMOTE_MIN_CONFIDENCE`
- non-empty evidence event ids
- source support from underlying transcript/observations/captured event
  content, not merely the Stop event id; if source expansion is unavailable
  or no source supports the claim, block with
  `summary_source_support_unavailable` or `summary_source_support_failed`
- `!contains_auto_promote_unsafe_marker(text)`
- risk class at most `medium` (the hardcoded summary constant); the gate does
  not require `low` because the constant makes that unsatisfiable by
  construction — the type allowlist and confidence floor replace the risk
  screen on this path

Block reasons mirror the check order with summary-specific entries
(`summary_type_not_allowlisted`, `summary_confidence_below_floor`, ...);
the shared reasons reuse the existing vocabulary.

### 3. Shadow mode (Phase 1)

Config key `promotion.summary_gate_mode = "off" | "shadow" | "enforce"`
(default `shadow` on first release). In shadow mode the gate is evaluated and
its verdict logged (`summary-gate: would_promote id=... reasons=[]`) and
counted, but the candidate is persisted as `pending_review`. A candidate that
would have promoted and is blocked only because shadow mode is active records
the explicit block reason `summary_gate_shadow`; candidates that fail a real
predicate keep the predicate-specific reason. A counter table or reuse of
`ai_usage_events`-style aggregate feeds the doctor line. Enforce mode performs
the same promotion call as the observation path (`promote_source_candidate` +
lifecycle update).

### 4. Observability

- `src/db/query/stats.rs` candidate-promotion query adds `source_kind` to the
  grouping and the 7-day split.
- Doctor: the promotion-funnel probe prints per-path rows and, in shadow
  mode, the would-promote count since enable. The declared-empty-surfaces
  probe pattern (#374) is the template.
- U-29: enforce-mode promotions and gate blocks log at the same level and
  shape as the observation path (`memory-candidate` target).

### 5. Confidence derivation (open question, resolved by Phase 1)

`SUMMARY_CANDIDATE_CONFIDENCE` (0.74) sits below the observation 0.80 floor.
Phase 1 therefore uses a 0.70 shadow floor or ships deterministic per-item
confidence derivation in the same phase. The shadow phase is not complete if
the would-promote cohort is zero solely because all allowlisted decision and
discovery summaries sit below the configured floor. Phase 1 telemetry must
answer whether to (a) raise per-item confidence for decision items carrying
strong signals (explicit decision verbs, file paths, commands), or (b) keep a
lower summary-specific floor with evidence. The flip to `enforce` is gated on
a recorded threshold decision in this file.

## Phases and Verification

Phase 1 (observability + shadow):
- migration + `source_kind` threading + gate in shadow + doctor/stats split
- tests: fixture summary candidate records `source_kind='summary'`; an
  ambiguous legacy candidate remains `source_kind='unattributed'`; supported
  decision/discovery candidates produce a non-zero would-promote cohort in
  shadow and record `summary_gate_shadow`; unsupported candidates record
  source-support block reasons; doctor split rendered
- verify: `cargo test memory_candidate`, `cargo test summary`,
  `cargo test doctor`

Phase 2 (enforce):
- flip default mode after thresholds recorded here; promotion fixture test
  (qualifying decision auto-promotes; lesson does not); real-session sampling
  recorded in the tracking issue for #381/#383
- verify: `cargo test`, manual `remem status` funnel delta on a live install

## Compatibility

- No changes to observation-path gates or thresholds.
- Existing pending rows are untouched; replay/backfill of the historical
  backlog stays behind explicit `remem pending` tooling (non-goal here).
- `finalize_summary_creates_candidates_without_active_memories` is updated to
  assert shadow-mode behavior instead of deleted (W-12: assertions move, not
  weaken).
