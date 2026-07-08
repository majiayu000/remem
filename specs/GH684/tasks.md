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

- [x] `SP684-T1` Owner: agent; Dependencies: spec approval; Done when: doctor/status reports legacy row counts, last-write epochs, and frozen-write violations for the tracked surfaces; Verify: doctor/status fixture tests.
- [x] `SP684-T2` Owner: maintainer or agent with approved fixtures; Dependencies: `SP684-T1`; Done when: field-level output comparison is established for `finalize_summarize` and `persist_session_rollup`; Verify: `summary_writer_equivalence_fixture_documents_field_level_deltas` documents legacy-only structured fields, rollup-only range fields, and the cooldown side-effect delta.
- [x] `SP684-T3` Owner: agent; Dependencies: `SP684-T2`; Done when: any load-bearing legacy Summary output delta is ported into SessionRollup; Verify: `summary_writer_equivalence_fixture_documents_field_level_deltas`, `session_rollup_structured_fields_feed_current_summary_readers`, `query_recent_summaries_uses_semantic_rollup_rows_without_synthetic_noise`, `recall_includes_semantic_rollup_session_but_excludes_synthetic_range_title`, and `overview_counts_semantic_rollup_summary_rows_as_sessions`.
- [x] `SP684-T4` Owner: agent; Dependencies: `SP684-T2`; Done when: Stop-hook side effects currently coupled to Summary are preserved or re-homed before `JobType::Summary` retirement; Verify: Compress, Dream, raw ingest, citation, failure lesson, candidate finalization, and native-memory tests. GH684-T4 locks the current side-effect owners before retirement: `enqueue_summary_followup_jobs_dedups_dream_and_preserves_profile_payload`, `enqueue_summary_followup_jobs_skips_legacy_summary_job`, `bad_transcript_path_uses_last_assistant_message_hook_fallback`, `process_records_memory_citations_before_cooldown_skip`, `process_records_memory_citations_before_summary_skip`, `process_distills_failure_lesson_before_cooldown_skip`, `finalize_summary_creates_candidates_without_active_memories`, and `process_finalized_summary_syncs_native_memory_side_effect`.
- [x] `SP684-T5` Owner: maintainer or release operator; Dependencies: `SP684-T1`; Done when: `pending_observations` emptiness is confirmed on real databases, or stragglers are migrated with `remem pending migrate-legacy`; Verify: GH-684 status comments `2026-07-08` confirm the default real database `/Users/apple/.remem/remem.db` and five dated `/Users/apple/Backups/remem/` stores have zero ready/delayed/processing/expired/failed rows; no migration was needed.
- [x] `SP684-T6` Owner: agent; Dependencies: `SP684-T5`; Done when: dead pending queue claim/write machinery is frozen or removed while admin migration/reporting stays available; Verify: pending admin/status tests plus production `cargo check` prove legacy enqueue/claim modules are removed from the crate while tests seed historical rows through `db::test_support::insert_legacy_pending_fixture`.
- [x] `SP684-T7` Owner: agent; Dependencies: `SP684-T2` `SP684-T3` `SP684-T4`; Done when: legacy `JobType::Summary` handling at upgrade is decided (drain, reject, or convert) and tested; Verify: migration v064 rejects non-terminal and retryable failed legacy Summary jobs as permanent failures, Stop hooks no longer enqueue new Summary jobs, capture-ledger failures spill instead of falling back to the retired writer, same-host/project/session stale spills are skipped after the current stop payload succeeds while other projects still replay, replayed Stop captures are idempotent after later replay-step failures, duplicate replay captures with the same fixed event ID do not revive completed rollup tasks, replay capture-ledger failures preserve one active spill row, raw/citation/failure side effects stay reachable from the hook path, citation recording failures log at error level without blocking follow-up jobs, doctor/status ignore v064 upgrade rejection rows as freeze blockers and actionable failed jobs while worker-side post-retirement Summary rejections stay visible, old-version daemon heartbeats and legacy singleton locks do not suppress the current Stop fallback worker, workers claim extraction tasks before Compress/Dream jobs, the worker rejects already-claimed Summary jobs before the retired path can run, terminal Summary history and non-summary jobs are preserved, and coverage includes `legacy_summary_upgrade_rejects_non_terminal_jobs`, `worker_rejects_legacy_summary_job_without_retry`, `summarize_hook_runs_stop_side_effects_without_summary_job`, `citation_failure_does_not_block_followup_jobs`, `replay_capture_failure_is_preserved_once_by_replay_layer`, `replay_capture_is_idempotent_when_later_followup_fails`, `duplicate_fixed_event_id_does_not_revive_done_task`, `current_healthy_daemon_skips_stop_spawn`, `old_version_healthy_daemon_uses_stop_fallback_spawn`, `once_bypasses_lock_for_old_version_daemon_heartbeat`, `old_version_daemon_lock_allows_current_once_heartbeat`, `summarize_hook_replays_same_session_spill_for_different_project`, `enqueue_summary_followup_jobs_skips_legacy_summary_job`, `capture_ledger_failure_blocks_followup_jobs`, `current_stop_payload_wins_over_same_session_spill_replay`, `upgrade_summary_rejections_are_not_actionable_but_worker_rejections_are`, and `legacy_surfaces_ignore_upgrade_summary_rejections_but_report_worker_rejections`.
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
