# Tech Spec

## Linked Issue

GH-674

## Accepted Contract

The authoritative technical contract is
`docs/specs/summary-promotion-gate/TECH.md`.

This SpecRail packet covers GH-690 Phase 1 and the current GH-696 Phase 2:
source-kind observability, shadow telemetry, and enforce-mode summary
promotion for supported factual summaries.

## Codebase Context

| Area | Files | Current behavior | Phase 1 change |
| --- | --- | --- | --- |
| Candidate schema | `src/migrations/`, `src/db/` | `memory_candidates` has no source-path column. | Add `source_kind` with `unattributed` default and deterministic writes for new rows. |
| Candidate persistence | `src/memory_candidate.rs` | Observation candidates can carry an auto-promote batch; summary candidates call `persist_candidate_rows(..., None)`. | Thread source kind from observation and summary callers. |
| Summary candidates | `src/memory/promote/summary.rs`, `src/summarize/summary_job/persist.rs` | Phase 1 evaluates summary decisions/discoveries in shadow mode. | Phase 2 defaults the gate to enforce mode while preserving shadow/off rollback modes. |
| Source support | `src/memory_candidate/support.rs`, capture/session data access | Observation support uses source observations. | Summary gate must verify against transcript, observations, or captured event content, not only Stop event ids. |
| Stats and diagnostics | `src/db/query/stats.rs`, `src/doctor/`, status command | Promotion funnel groups by status and block reason only. | Split by source kind and expose summary shadow would-promote count. |
| Tests | existing migration, candidate, summary, doctor/status tests | Existing summary pin asserts no active memories. | Update assertions to Phase 1 shadow behavior without weakening coverage. |

## Design Rules

- Fail closed: missing or unloadable source evidence leaves the candidate
  pending and records a source-support block reason.
- Shadow is explicit: a would-promote summary candidate records
  `summary_gate_shadow`, not `unknown`.
- Enforce is explicit: supported summary candidates promote only in
  `promotion.summary_gate_mode = "enforce"` and only after the same
  summary-specific verdict that shadow mode reports.
- Observation-path behavior must remain unchanged except for source-kind
  attribution.
- The Phase 2 floor is 0.70 for summary decisions/discoveries; this exercises
  current 0.74 summary candidates without lowering the observation-path floor.

## Verification Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| Source-kind default and writes | migration + persistence | migration test and candidate persistence tests |
| Shadow would-promote path | summary gate | summary fixture test for `summary_gate_shadow` |
| Source support required | summary support lookup | unsupported-source fixture test |
| Enforce promotion path | summary gate + lifecycle apply | summary fixture test for `auto_promoted` decision and active memory |
| Diagnostics split | stats + doctor/status | doctor/status rendering tests |
| SpecRail packet valid | `specs/GH674/` | `python3 checks/check_workflow.py --repo . --spec-dir specs/GH674` |

## Rollback

Set summary gate mode to `shadow` to stop automatic summary promotion while
preserving would-promote telemetry, or `off` to skip summary-gate evaluation.
The additive `source_kind` column and diagnostics split remain useful
observability.
