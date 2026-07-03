# Task Plan

## Linked Issue

GH-674

## Implementation Issues

- GH-690: Phase 1 shadow observability (merged)
- GH-696: Phase 2 enforce mode and sampling evidence

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/summary-promotion-gate/PRODUCT.md` and
  `docs/specs/summary-promotion-gate/TECH.md`

## Tasks

- [ ] `SP674-T1` Owner: agent; Dependencies: none; Done when: `memory_candidates.source_kind` exists with `unattributed` default and new candidate writes set deterministic `observation` or `summary` values; Verify: migration and persistence tests.
- [ ] `SP674-T2` Owner: agent; Dependencies: `SP674-T1`; Done when: summary decision/discovery candidates are evaluated by a Phase 1 shadow gate and would-promote candidates record `summary_gate_shadow` while staying `pending_review`; Verify: summary fixture tests.
- [ ] `SP674-T3` Owner: agent; Dependencies: `SP674-T2`; Done when: unsupported summary candidates record `summary_source_support_unavailable` or `summary_source_support_failed`; Verify: unsupported-source fixture tests.
- [ ] `SP674-T4` Owner: agent; Dependencies: `SP674-T1` `SP674-T2`; Done when: stats, doctor, and status output split promotion counters by source kind and report summary shadow would-promote counts; Verify: doctor/status tests.
- [ ] `SP674-T5` Owner: agent; Dependencies: `SP674-T2` `SP674-T3`; Done when: `promotion.summary_gate_mode` defaults to `enforce`, while `shadow` and `off` remain configurable rollback modes; Verify: runtime config tests.
- [ ] `SP674-T6` Owner: agent; Dependencies: `SP674-T5`; Done when: a supported summary-derived decision/discovery auto-promotes in enforce mode and the same candidate records `summary_gate_shadow` in shadow mode; Verify: summary gate fixture tests.
- [ ] `SP674-T7` Owner: agent; Dependencies: `SP674-T5`; Done when: summary-derived lessons/preferences and unsupported summary candidates remain pending with explicit block reasons; Verify: summary fixture tests.
- [ ] `SP674-T8` Owner: agent; Dependencies: `SP674-T1` `SP674-T2` `SP674-T3` `SP674-T4` `SP674-T5` `SP674-T6` `SP674-T7`; Done when: local deterministic checks and focused Rust tests pass and sampling evidence is posted to GH-674; Verify: commands below.

## Parallel Split

No parallel writable lanes. The schema, persistence, summary gate, and
diagnostics changes share the candidate model. Phase 2 also touches the same
summary-gate path, so it should land as one implementation PR.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH674`
- `cargo fmt --check`
- `cargo check`
- Focused Rust tests for migrations, memory candidates, summary persistence,
  doctor/status stats
- `cargo test` before merge readiness

## Handoff Notes

Phase 1 PR used `Refs #674` and closed GH-690. Phase 2 PR should use
`Refs #674` and close GH-696. Do not close GH-674 until Phase 2 enforcement and
real-session sampling evidence are complete.
