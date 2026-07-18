# Task Plan

## Linked Issue

GH-720

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [x] `SP720-T1` Owner: implementation agent; Dependencies: `raw_messages` archive layer and transcript parser; Done when: `remem ingest-sessions` scans Claude/Codex JSONL roots, supports explicit `--root label=path`, records per-file cursors, reports machine-readable summaries, isolates per-file and discovery failures, and preserves source-root labels; Verify: PR #725, issue #722, `src/ingest/sessions.rs`, `src/ingest/sessions/tests.rs`, and the GH720 streaming follow-up. PR #725 supplied discovery, cursors, summaries, isolation, and source-root identity; the follow-up streams JSONL records with bounded memory and rolls back a file's savepoint on later read/UTF-8 failure.
- [x] `SP720-T2` Owner: implementation agent; Dependencies: `SP720-T1`; Done when: raw rows keep transcript event timestamps, source roots participate in durable deduplication, and all-project time-window session queries have a created-at-leading index; Verify: PR #725, `src/memory/raw_archive.rs`, `src/memory/raw_transcript.rs`, `src/migrations/v055_session_ingest_cursors.sql`, and `src/migrations/v056_raw_messages_source_root_key.sql`. Merged: migrations `v055`/`v056` and the raw archive/transcript changes are on `main`.
- [x] `SP720-T3` Owner: implementation agent; Dependencies: `SP720-T2`; Done when: raw search accepts `since`/`until`, session listing groups by `(source_root, project, session_id)`, samples per-session user messages, and CLI/MCP JSON shapes match; Verify: PR #725, issue #723, raw archive tests, and CLI/MCP command wiring. The GH720 follow-up adds a shared `memory::raw_query` JSON contract for CLI/MCP raw search, preserves pagination/full row content, and makes date-only `until` include the full UTC day, with focused raw archive and MCP regression tests.
- [x] `SP720-T4` Owner: maintainer or release operator with access to real local transcripts; Dependencies: `SP720-T1` `SP720-T2` `SP720-T3`; Done when: `remem raw sessions --since <window> --json` is compared against the existing recap script for the same window, and differences in session/message counts are recorded on GH-720; Verify: GH-871's aggregate-only fixed-window release handoff in `artifacts/logs/gh871/fixed-window-reconciliation.json` compares the shared transcript classifier with the raw archive for `1783653658..1784258459`: 293 sessions and 13,443 messages (1,403 user, 12,040 assistant) match exactly, all mismatch/conflict/event-time failure counts are zero, and parity is true. The GH-871 implementation PR carries `Closes #871`; no GitHub state or comment was changed on GH-720.
- [ ] `SP720-T5` Owner: future implementation agent in the refine repository; Dependencies: `SP720-T4`; Done when: refine reads remem raw query output as its only input source and posts a before/after facet reconciliation report; Verify: refine PR plus GH-720 reconciliation comment.
- [ ] `SP720-T6` Owner: future spec or implementation agent; Dependencies: `SP720-T5`; Done when: facets table, job type, LLM usage accounting, and backfill cap receive an independent tech review before implementation; Verify: follow-up spec or GH-720 review note.

## Parallelization

- SP720-T1 and SP720-T2 are tightly coupled through raw archive insertion and should be serialized.
- SP720-T3 can be reviewed in parallel after SP720-T2 is stable; writable files are raw query/CLI/MCP surfaces only.
- SP720-T4 is manual verification and can run after the Phase 1 PR is locally available.
- SP720-T5 and SP720-T6 are future phases and must not share writable files with Phase 1.

## Verification

- `cargo fmt --check`
- `cargo check`
- `cargo test ingest::sessions -- --nocapture`
- `cargo test raw_transcript -- --nocapture`
- `cargo test`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH720`
- Fresh PR #725 CI `check`
- PR #725 review-thread and merge-state gate

## Handoff Notes

- PR #725 is the Phase 1 implementation PR for GH-720 and closes GH-722/GH-723.
- GH-722/GH-723 are umbrella-covered by this GH-720 packet; no separate `specs/GH722` or `specs/GH723` packet is expected for Phase 1.
- Phase 2 and Phase 3 remain open after PR #725 unless explicitly implemented in later PRs.
- Status sync 2026-07-10: PR #725 merged and implementation issues #722/#723 are closed. SP720-T2 has sufficient completion evidence; SP720-T1 and SP720-T3 remain open for the bounded-memory and CLI/MCP/date-boundary acceptance gaps above. Phase 1 acceptance also remains pending SP720-T4, the manual maintainer comparison against the recap script on real transcripts. Phase 2/3 remain deferred in SP720-T5/T6.
- Status sync 2026-07-16: the GH720 query-parity follow-up completes SP720-T3 with a shared CLI/MCP raw-search envelope and inclusive date-only `until` semantics. SP720-T1 remains open for bounded-memory transcript draining, and SP720-T4 still requires a maintainer-owned comparison against real local transcripts before Phase 1 acceptance; SP720-T5/T6 remain deferred behind that evidence.
- Status sync 2026-07-16: the streaming-ingest follow-up completes SP720-T1 by replacing whole-file transcript reads on the archive drain path with bounded JSONL streaming and transactional rollback on read failure. SP720-T4 still requires the maintainer-owned real-transcript comparison before Phase 1 acceptance; SP720-T5/T6 remain deferred behind that evidence.
- Status sync 2026-07-19: GH-871 completes SP720-T4 with the sanitized fixed-window reconciliation artifact. Transcript and raw occurrence multisets match for all 293 selected sessions and 13,443 messages; XML/control and unsupported records are reported as intentional aggregate exclusions, while every strict parity blocker is zero. SP720-T5/T6 remain deferred and no GitHub state or comment was changed on GH-720.
