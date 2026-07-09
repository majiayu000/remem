# Task Plan

## Linked Issue

GH-720

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [x] `SP720-T1` Owner: implementation agent; Dependencies: `raw_messages` archive layer and transcript parser; Done when: `remem ingest-sessions` scans Claude/Codex JSONL roots, supports explicit `--root label=path`, records per-file cursors, reports machine-readable summaries, isolates per-file and discovery failures, and preserves source-root labels; Verify: PR #725, issue #722, `src/ingest/sessions.rs`, and `src/ingest/sessions/tests.rs`. Merged: PR #725 landed `src/cli/actions/ingest_sessions.rs`, `src/ingest/sessions.rs`, and `src/ingest/sessions/tests.rs` on `main`; impl issue #722 is closed.
- [x] `SP720-T2` Owner: implementation agent; Dependencies: `SP720-T1`; Done when: raw rows keep transcript event timestamps, source roots participate in durable deduplication, and all-project time-window session queries have a created-at-leading index; Verify: PR #725, `src/memory/raw_archive.rs`, `src/memory/raw_transcript.rs`, `src/migrations/v055_session_ingest_cursors.sql`, and `src/migrations/v056_raw_messages_source_root_key.sql`. Merged: migrations `v055`/`v056` and the raw archive/transcript changes are on `main`.
- [x] `SP720-T3` Owner: implementation agent; Dependencies: `SP720-T2`; Done when: raw search accepts `since`/`until`, session listing groups by `(source_root, project, session_id)`, samples per-session user messages, and CLI/MCP JSON shapes match; Verify: PR #725, issue #723, raw archive tests, and CLI/MCP command wiring. Merged: raw time-window query surface landed via PR #725; impl issue #723 is closed.
- [ ] `SP720-T4` Owner: maintainer or release operator with access to real local transcripts; Dependencies: `SP720-T1` `SP720-T2` `SP720-T3`; Done when: `remem raw sessions --since <window> --json` is compared against the existing recap script for the same window, and differences in session/message counts are recorded on GH-720; Verify: GH-720 status comment or release handoff.
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
- Status sync 2026-07-09: Phase 1 (SP720-T1..T3) is verified complete on `main` — PR #725 merged and impl issues #722/#723 are closed. Remaining work is SP720-T4 (manual maintainer verification against the recap script on real transcripts) and the deferred Phase 2/3 items SP720-T5/T6.
