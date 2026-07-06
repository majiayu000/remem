# Task Plan

## Linked Issue

GH-684

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/legacy-observation-retirement/PRODUCT.md` and
  `docs/specs/legacy-observation-retirement/TECH.md`

## Current Status

Phase 1 static inventory and dogfood evidence were recorded in PR #686 and in
the issue status comment. The remaining work is not a wholesale observation
rewrite; it is focused convergence of one dead surface, one duplicate writer
chain, and one mislabeled current surface.

## Implementation Tasks

- [ ] `SP684-T1` Owner: agent; Dependencies: spec approval; Done when: doctor/status reports legacy row counts, last-write epochs, and frozen-write violations for the tracked surfaces; Verify: doctor/status fixture tests.
- [ ] `SP684-T2` Owner: maintainer or agent with approved fixtures; Dependencies: `SP684-T1`; Done when: field-level output equivalence is established for `finalize_summarize` and `persist_session_rollup`; Verify: committed equivalence fixtures.
- [ ] `SP684-T3` Owner: agent; Dependencies: `SP684-T2`; Done when: any load-bearing legacy Summary output delta is ported into SessionRollup; Verify: context/timeline/user-context regression tests.
- [ ] `SP684-T4` Owner: agent; Dependencies: `SP684-T2`; Done when: Stop-hook side effects currently coupled to Summary are preserved or re-homed before `JobType::Summary` retirement; Verify: Compress, Dream, raw ingest, citation, failure lesson, candidate finalization, and native-memory tests.
- [ ] `SP684-T5` Owner: maintainer or release operator; Dependencies: `SP684-T1`; Done when: `pending_observations` emptiness is confirmed on real databases, or stragglers are migrated with `remem pending migrate-legacy`; Verify: GH-684 status comment or release handoff.
- [ ] `SP684-T6` Owner: agent; Dependencies: `SP684-T5`; Done when: dead pending queue claim/write machinery is frozen or removed while admin migration/reporting stays available; Verify: pending admin and status tests.
- [ ] `SP684-T7` Owner: agent; Dependencies: `SP684-T2` `SP684-T3` `SP684-T4`; Done when: legacy `JobType::Summary` handling at upgrade is decided (drain, reject, or convert) and tested; Verify: upgrade/migration tests.
- [x] `SP684-T8` Owner: agent; Dependencies: none after spec approval; Done when: MCP and docs stop describing live `observations` as legacy; Verify: docs or descriptor tests.
- [ ] `SP684-T9` Owner: agent; Dependencies: deprecation window; Done when: guarded drop migration refuses to run while unmigrated valuable rows remain; Verify: migration refusal and schema-drift tests.

## Parallelization

Doctor visibility and wording fixes can proceed independently. Summary
equivalence, side-effect preservation, and Summary job retirement must stay
serial because they touch the same Stop-hook behavior.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH684`
- `cargo fmt --check`
- `cargo check`
- Focused doctor, pending, summary equivalence, Stop-hook, and migration tests
- `cargo test` before merge readiness

## Handoff Notes

Use `Refs #684` for spec-only or partial implementation PRs. Do not close
GH-684 until every acceptance criterion in `product.md` is implemented and
verified. Summary writer retirement requires explicit equivalence evidence and
human review before merge.
