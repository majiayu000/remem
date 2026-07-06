# Task Plan

## Linked Issue

GH-678

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/project-memory-pack/PRODUCT.md` and
  `docs/specs/project-memory-pack/TECH.md`

## Implementation Tasks

- [x] `SP678-T1` Owner: agent; Dependencies: spec approval; Done when: the pack manifest and canonical JSONL format are implemented behind export-only code paths; Verify: serializer unit tests.
- [x] `SP678-T2` Owner: agent; Dependencies: `SP678-T1`; Done when: `remem export --project --pack` writes deterministic `pack.json`, `memories.jsonl`, and derived `INDEX.md`; Verify: unchanged export byte equality test.
- [x] `SP678-T3` Owner: agent; Dependencies: `SP678-T2`; Done when: export filters only active repo-owned startup memories and reruns redaction with fail-loud errors; Verify: filtering and seeded-secret tests.
- [x] `SP678-T4` Owner: agent; Dependencies: `SP678-T1`; Done when: import dry-run validates manifest/digest and reports add/dedup/skip/conflict/quarantine categories without mutation; Verify: planner tests. Completed by the GH678 dry-run planner tranche.
- [ ] `SP678-T5` Owner: agent; Dependencies: `SP678-T4`; Done when: import never resurrects suppressed or inactive local decisions; Verify: suppression and invalidation tests.
- [ ] `SP678-T6` Owner: agent; Dependencies: `SP678-T4`; Done when: export -> fresh-store import -> export is byte-identical; Verify: round-trip fixture.
- [ ] `SP678-T7` Owner: agent; Dependencies: GH-672 trust-class implementation; Done when: active import writes `pack` trust class and runs instruction-pattern scan before insertion; Verify: trust and quarantine tests.
- [ ] `SP678-T8` Owner: agent; Dependencies: `SP678-T7`; Done when: doctor and `remem why` expose pack origin counts and source attribution; Verify: doctor/why tests.
- [ ] `SP678-T9` Owner: agent; Dependencies: export/import behavior; Done when: README team-onboarding walkthrough matches actual CLI behavior; Verify: docs review and command smoke.

## Parallelization

Export serialization and import planner can proceed in separate lanes after
the format is fixed. Active import must wait for GH-672 trust-class and
quarantine support.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH678`
- `cargo fmt --check`
- `cargo check`
- Focused export/import/redaction/planner/doctor tests
- `cargo test` before merge readiness

## Handoff Notes

Use `Refs #678` for spec-only or partial implementation PRs. Do not close
GH-678 until every acceptance criterion in `product.md` is implemented and
verified. Active import is blocked on GH-672's approved trust-class contract.
