# Tech Spec

## Linked Issue

GH-674

## Accepted Contract

The authoritative technical contract is
`docs/specs/summary-promotion-gate/TECH.md`.

This SpecRail packet narrows the implementation to GH-690 Phase 1: source-kind
observability plus shadow summary gate telemetry.

## Codebase Context

| Area | Files | Current behavior | Phase 1 change |
| --- | --- | --- | --- |
| Candidate schema | `src/migrations/`, `src/db/` | `memory_candidates` has no source-path column. | Add `source_kind` with `unattributed` default and deterministic writes for new rows. |
| Candidate persistence | `src/memory_candidate.rs` | Observation candidates can carry an auto-promote batch; summary candidates call `persist_candidate_rows(..., None)`. | Thread source kind from observation and summary callers. |
| Summary candidates | `src/memory/promote/summary.rs`, `src/summarize/summary_job/persist.rs` | Summary candidates hardcode medium risk and confidence 0.74; they never promote. | Evaluate summary decisions/discoveries in shadow mode and record path-aware block reasons. |
| Source support | `src/memory_candidate/support.rs`, capture/session data access | Observation support uses source observations. | Summary gate must verify against transcript, observations, or captured event content, not only Stop event ids. |
| Stats and diagnostics | `src/db/query/stats.rs`, `src/doctor/`, status command | Promotion funnel groups by status and block reason only. | Split by source kind and expose summary shadow would-promote count. |
| Tests | existing migration, candidate, summary, doctor/status tests | Existing summary pin asserts no active memories. | Update assertions to Phase 1 shadow behavior without weakening coverage. |

## Design Rules

- Fail closed: missing or unloadable source evidence leaves the candidate
  pending and records a source-support block reason.
- Shadow is explicit: a would-promote summary candidate records
  `summary_gate_shadow`, not `unknown`.
- Observation-path behavior must remain unchanged except for source-kind
  attribution.
- The Phase 1 floor must exercise current decision/discovery summaries at 0.74
  or ship deterministic confidence derivation in the same slice.

## Verification Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| Source-kind default and writes | migration + persistence | migration test and candidate persistence tests |
| Shadow would-promote path | summary gate | summary fixture test for `summary_gate_shadow` |
| Source support required | summary support lookup | unsupported-source fixture test |
| Diagnostics split | stats + doctor/status | doctor/status rendering tests |
| SpecRail packet valid | `specs/GH674/` | `python3 checks/check_workflow.py --repo . --spec-dir specs/GH674` |

## Rollback

Set summary gate mode to `off` to stop shadow evaluation. The additive
`source_kind` column and diagnostics split remain useful observability and do
not promote candidates.
