# Task Plan

## Linked Issue

GH-680

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/procedure-skill-export/PRODUCT.md` and
  `docs/specs/procedure-skill-export/TECH.md`

## Implementation Tasks

- [x] `SP680-T1` Owner: agent; Dependencies: spec approval; Done when: `remem procedures list` shows active procedure memories with maturity and freshness signals; Verify: CLI and JSON tests. Shipped by PR #744.
- [x] `SP680-T2` Owner: agent; Dependencies: `SP680-T1`; Done when: export eligibility reuses procedure promotion/freshness evidence and rejects ineligible rows; Verify: eligibility tests. Shipped by this partial implementation slice.
- [x] `SP680-T3` Owner: agent; Dependencies: `SP680-T2`; Done when: all rendered fields are scanned before file writes; Verify: secret and instruction-pattern rejection tests. Shipped by this partial implementation slice.
- [x] `SP680-T4` Owner: agent; Dependencies: `SP680-T2` `SP680-T3`; Done when: templates render `claude-skill`, `codex-prompt`, and `runbook-md` drafts with provenance; Verify: snapshot tests. Shipped by this partial implementation slice.
- [x] `SP680-T5` Owner: agent; Dependencies: `SP680-T4`; Done when: writer refuses high-context paths and user-edited existing files, and allows only explicit generated-overwrite cases; Verify: path and overwrite tests. Shipped by this partial implementation slice.
- [x] `SP680-T6` Owner: agent; Dependencies: `SP680-T5`; Done when: worker, dream, hook, and MCP write paths cannot reach the export writer; Verify: negative reachability test. Shipped by this partial implementation slice.
- [x] `SP680-T7` Owner: agent; Dependencies: `SP680-T4`; Done when: `procedure_exports` registry records source snapshot and doctor flags inactive, stale, or changed source procedures; Verify: migration and doctor tests. Shipped by this partial implementation slice.
- [x] `SP680-T8` Owner: agent; Dependencies: export behavior; Done when: `docs/procedural-memory.md` documents the review-gated export contract; Verify: docs review. Shipped by this final implementation slice.
- [x] `SP680-T9` Owner: agent; Dependencies: all implementation tasks; Done when: local verification passes; Verify: `cargo fmt --check`, `cargo check`, focused tests, and `cargo test`. Shipped by this final implementation slice.

## Parallelization

Listing and template rendering can split after eligibility is defined. Writer
guard and doctor registry touch security-sensitive surfaces and should stay
serial or use disjoint file ownership.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH680`
- `cargo fmt --check`
- `cargo check`
- Focused procedure CLI/export/template/doctor tests
- `cargo test` before merge readiness

## Handoff Notes

Use `Refs #680` for spec-only or partial implementation PRs. Do not close
GH-680 until every acceptance criterion in `product.md` is implemented and
verified. This packet touches high-context file generation and requires human
spec approval before runtime implementation.
