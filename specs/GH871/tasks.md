# Task Plan

## Linked Issue

GH-871

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [ ] `SP871-T1` Owner: implementation coordinator; Dependencies: accepted `product.md` and `tech.md`; Done when: v071 adds the declared `raw_session_identities` ledger, current/legacy project aliases, transcript event-range/cursor index, and raw event-time provenance with schema-drift coverage; Stop and batch ingestion share metadata-first identity; batch Phase A persists every claim before Phase B mutation; version-0 rows refresh transactionally; unambiguous legacy-project/fallback rows rekey in place; canonical collisions compare stable fields, normalize volatile hook metadata, preserve/rewrite every raw-message evidence reference, and remain rollback-safe/idempotent; persisted conflicts stay sticky and error-visible; Verify: `cargo test -q ingest::sessions --lib`, focused Stop identity, two-phase ordering, legacy-project, evidence-reference, v071 migration, and provenance upgrade/rollback/retry tests, `cargo test -q migrate::tests_schema_drift --lib`, and `git diff --check`.
- [ ] `SP871-T2` Owner: implementation coordinator; Dependencies: `SP871-T1`; Done when: `raw search` and `raw sessions` use the validated no-write `open_db_read_only_current()` path, a held `BEGIN IMMEDIATE` no longer blocks either CLI read, stale/drifted schemas fail with the migration diagnostic without writes, and session JSON adds correct `user_message_count` and `assistant_message_count` without removing or renaming existing fields; Verify: focused raw CLI lock/stale-schema tests, `cargo test -q raw_archive --lib`, `cargo test -q cli::tests_raw --lib`, and `git diff --check`.
- [ ] `SP871-T3` Owner: implementation coordinator; Dependencies: `SP871-T1` and `SP871-T2`; Done when: the shared transcript classifier and `remem raw reconcile --since --until [--root] --json` validate `since <= until` plus a current ledger, select only event-range-intersecting transcript files, compare internal per-message identity multisets, query durable conflicts, distinguish archive ingest-fallback from legacy-unknown time, and produce deterministic aggregate-only output; missing required roots, stale indexes, and conflicts fail loudly, and sensitive fixture sentinels never appear in output; Verify: `cargo test -q raw_transcript --lib`, `cargo test -q raw_reconcile --lib`, equal-count substitution/persisted-conflict/inverted-window/bounded-read tests, CLI parse/JSON snapshot tests, and `git diff --check`.
- [ ] `SP871-T4` Owner: implementation coordinator with access to the local fixed-window corpus; Dependencies: `SP871-T1` `SP871-T2` `SP871-T3`; Done when: README, architecture, the current raw-session contract, its index entry, and GH720 task evidence match the shipped behavior; `remem ingest-sessions --json` followed by `remem raw reconcile --since 1783653658 --until 1784258459 --json` records only sanitized aggregates; every non-parity count is explained; and the final GH-871 implementation PR uses `Closes #871`; Verify: sanitized artifact review, `python3 checks/check_workflow.py --repo . --spec-dir specs/GH871`, full repository preflight, PR CI, independent reviewer lane, GraphQL review-thread state, and `pr_gate`.

## Parallelization

- Production and migration writes are single-lane because identity probing,
  schema state, rekey behavior, session aggregation, and reconciliation share
  one acceptance surface.
- Read-only planner and reviewer lanes may inspect the issue packet, diff, and
  evidence without running Cargo or editing files.
- The coordinator is the exclusive Cargo/test owner for this worktree.
- `SP871-T2` follows the stable identity/schema contract from `SP871-T1`;
  `SP871-T3` follows both; `SP871-T4` runs only after the final behavior is
  fixed.

## Verification

- `python3 checks/route_gate.py --repo . --route implement --issue 871 --state ready_to_implement --duplicate-evidence artifacts/logs/gh871/duplicate-work-evidence.json --json`
- `cargo fmt --check`
- `cargo check`
- Focused tests named in `SP871-T1` through `SP871-T3`
- `cargo test`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH871`
- `python3 scripts/ci/check_plugin_version_sync.py`
- `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/gh871-pr-body.md`
- `gh pr checks <n> --repo majiayu000/remem --watch --fail-fast`
- Independent native reviewer/merge-reviewer evidence for the current PR head
- Current-head GraphQL review threads, merge state, and allowed `pr_gate`

## Handoff Notes

- Queue scope is only issue #871. The GH720 packet is updated solely because
  #871 acceptance explicitly requires parent task evidence; do not comment on,
  close, label, or otherwise mutate GitHub issue #720 in this tranche.
- The implementation is `pr_tier: heavy`: merge a spec-only PR with `Refs
  #871` before opening the final mixed implementation PR with `Closes #871`.
- Raw archive completeness is authoritative. Meta/XML conversational
  exclusions are metrics, never deletion rules.
- Reconciliation output may contain aggregate integers and fixed policy/window
  metadata only. Paths, project names, message text, full IDs, and hashes are
  prohibited.
- A committed task packet never supplies merge authorization. For this run,
  the runtime checkpoint must cite the user's current `implx auto` invocation
  as standing authorization; later runs must independently record their own
  current-conversation authorization. Fresh CI, reviewer, review-thread,
  merge-state, runtime-ledger, and PR-gate evidence remain mandatory.
