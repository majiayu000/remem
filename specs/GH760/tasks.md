# GH760 Task Plan

## Linked Issue

GH-760

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## 实现任务

- [x] `SP760-T1` Owner: CLI/user-context implementer; Dependencies: none; Done when: `remem user backfill [--apply] [--json] [--limit <n>]` is wired through the existing CLI command tree with dry-run as the default; Verify: focused CLI parsing tests and dry-run command tests. Shipped in partial implementation PR for GH760; apply fails closed until `SP760-T2`.
- [x] `SP760-T2` Owner: user-context storage implementer; Dependencies: `SP760-T1`; Done when: visible active user-scope preference memories are selected with the same expiry/suppression filters as current readers, classified through non-retention and sensitivity guards, converted to `Preference` claims with `source_kind = "preference_backfill"` and JSON-array memory source refs, and source memory rows remain unchanged; Verify: focused storage/unit tests for selection, conversion, source-ref parsing, and source immutability. Shipped in partial implementation PR for GH760; summary/recall dedupe remains in `SP760-T4`.
- [x] `SP760-T3` Owner: idempotency/reporting implementer; Dependencies: `SP760-T2`; Done when: claim-key and source-ref duplicate checks include governed claim rows, repeated apply runs add zero rows, `--json` exposes stable `converted[{memory_id, claim_id}]` report fields, and text-too-long/non-retention/project-scope/governed/sensitivity skips report explicit reasons; Verify: idempotency, governed duplicate, skip reason, `--limit`, and JSON snapshot tests. Shipped in partial implementation PR for GH760; final docs/release remain in `SP760-T5`.
- [ ] `SP760-T4` Owner: summary/recall implementer; Dependencies: `SP760-T2`; Done when: user summary/profile/recall source collection does not include the same backfilled preference as both a legacy memory and a claim; Verify: integration test covering backfilled preference summary sources.
- [ ] `SP760-T5` Owner: docs/release implementer; Dependencies: `SP760-T1`, `SP760-T2`, `SP760-T3`, `SP760-T4`; Done when: README or CLI help/release notes explain dry-run default, apply semantics, source traceability, visible-memory filtering, skip reasons, and rollback/governance; Verify: `python3 scripts/ci/check_pr_preflight.py --fast --base origin/main --pr-body-file <body>`.

## 并行拆分

- `SP760-T1`, `SP760-T2`, `SP760-T3`, and `SP760-T4` all touch `src/user_context/**` or CLI command surfaces and should run serially unless a first PR exposes a stable internal API.
- `SP760-T5` can be planned read-only in parallel, but documentation edits should wait until command flags and report fields are final.
- Do not combine GH-760 implementation with GH-759 policy changes; GH-760 is deterministic historical backfill, while GH-759 changes ongoing auto-promote behavior.

## 验证

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH760`
- Focused Rust tests named by each implementation PR.
- `cargo fmt --check`
- `cargo check`
- `cargo test` before final closure.

## Handoff Notes

- Spec-only PRs use `Refs #760`; implementation PRs use `Refs #760` until every task above is complete.
- Do not close GH-760 until dry-run, apply, idempotency, source traceability, non-retention skips, JSON report, docs, and full verification have landed.
- GH-759 complements this work for future automatic promotion, but GH-760 must remain deterministic and user-triggered.
