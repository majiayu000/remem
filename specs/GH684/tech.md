# Tech Spec

## Linked Issue

GH-684

## Product Spec

Link to `product.md`.

## Accepted Contract

The authoritative technical contract is
`docs/specs/legacy-observation-retirement/TECH.md`.

This SpecRail packet reflects the existing #684 contract and keeps
implementation behind the normal SpecRail readiness and spec-approval gates.

## Codebase Context

| Surface | Files | Verified status | Implementation concern |
| --- | --- | --- | --- |
| `pending_observations` | `src/db/pending/` | No default-path writer; dogfood queue empty. | Retire dead queue machinery only after real-db confirmation and migration escape hatch. |
| `observations` | `src/observation_extract.rs`, `src/db/observation.rs`, MCP/context/timeline readers | Live current intermediate. | GH684-T8 fixed misleading legacy wording; do not retire. |
| `observations_fts` | migrations triggers, timeline anchor search | Current trigger-maintained search index. | Keep with `observations`. |
| `session_summaries` | `src/session_rollup/`, `src/db/summarize/session/`, context/timeline/user-context readers | Load-bearing table with duplicate writers. | Retire legacy Summary job chain, not the table. |
| Stop hook side effects | `src/summarize/summary_job/`, `src/session_rollup/`, `src/worker.rs` | Summary path formerly owned other behaviors. | Preserve Compress/Dream/raw/citation/failure/candidate/native-memory side effects by re-homing each to Stop capture or SessionRollup worker side effects before removal. |

## Design Rules

- Reads move before writes die.
- Freeze states are observable in doctor.
- Drop migrations are guarded and refuse silent data loss.
- `observations` wording changes are accuracy fixes, not deprecations.
- Summary retirement is gated by field-level equivalence fixtures.

## Proposed Design

1. Keep the verified writer/reader inventory in the authoritative TECH spec.
2. Add doctor/status visibility for legacy row counts, last writes, and frozen
   write violations.
3. Add `finalize_summarize` versus `persist_session_rollup` equivalence
   fixtures for seeded sessions.
4. Port any load-bearing delta from the legacy Summary path into SessionRollup.
5. Preserve or re-home every non-summary Stop-hook side effect before removing
   `JobType::Summary`.
6. Confirm `pending_observations` emptiness across real databases and keep
   `pending migrate-legacy` as the explicit migration path.
7. Keep MCP and architecture docs from describing live observations as legacy.
8. Ship any table drop only after a deprecation window and a guarded migration.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| Inventory remains factual | docs/specs contract | spec update review |
| Legacy state visible | doctor/status | fixture DB doctor tests |
| Summary equivalence | summarize/session_rollup | row-shape/content fixture tests |
| Stop side effects preserved | Stop hook / worker | regression tests for Compress, Dream, raw, citation, candidates, native memory |
| Pending queue safe retire | pending admin/migrations | real-db confirmation plus migration tests |
| Wording fixed | MCP/docs | description tests or docs review |
| Drops guarded | migrate/schema drift | guarded-drop refusal tests |

## Data Flow

Current capture writes `captured_events`, extraction tasks, observations,
memory candidates, memories, raw messages, and SessionRollup summaries. The
legacy Summary job chain duplicates session summary writes and related side
effects. The convergence path proves the current SessionRollup path owns all
needed fields and side effects, then removes only the redundant Summary writer.

## Risks

- Data loss: mitigated by reads-before-writes, equivalence fixtures, and
  guarded migrations.
- Behavior regression: context, timeline, user-context recall, and `why`
  depend on `session_summaries`; the table stays.
- Operational drift: legacy failed jobs may remain; cleanup must be explicit
  rather than hidden in migration.

## Test Plan

- [ ] Doctor/status fixtures for counts, last-write epochs, and frozen writes.
- [x] `finalize_summarize` versus `persist_session_rollup` field-comparison
      fixture: `summary_writer_equivalence_fixture_documents_field_level_deltas`
      documents legacy-only structured fields, rollup-only range fields, and
      the legacy cooldown side-effect delta. GH684-T3 updates the fixture so
      SessionRollup owns the load-bearing request, decisions, learned,
      next_steps, and preferences fields while cooldown remains a separate
      retirement side effect.
- [x] Context, timeline, and user-context regression tests prove semantic
      rollup rows feed summary readers while synthetic `Captured event range`
      fallback titles stay hidden from user-facing context.
- [ ] Stop-hook side-effect regression tests cover the implemented ownership
      split before `JobType::Summary` retirement: the Stop hook owns immediately
      available memory citations and failure lessons after capture, while
      SessionRollup worker side effects own byte-bounded raw archive ingest plus
      transcript-only citations/failure lessons, summary-derived candidate
      finalization, workstream upsert, native memory sync, UserContextCandidate
      extraction, and Compress/Dream enqueue after rollup persistence. Raw
      archive ingest covers every Stop payload coalesced into the claimed range,
      deduplicates repeated transcript paths, and summary-derived candidate
      evidence is sourced from that exact range rather than the session-wide
      latest capture. The #794 follow-up passes the same selected, byte-bounded
      user/assistant transcript messages into the summarizer and candidate
      support text, removes exact captured-event duplicates, and applies the
      count, byte, and redaction budget once before either consumer. Migration
      v066 persists that exact-range evidence plus raw archive completion so a
      persisted-rollup retry does not reread an already-drained source file.
      Each bounded Stop with assistant evidence also persists the final
      message hash and structured citation facts outside the lossy prompt slice,
      including every boundary of a repeated path. Early v066 JSON replays its
      original bounded message hash for upgrade idempotency, preserving
      citation replay after per-message/global eviction or source deletion. A
      legacy Stop without a boundary skips transcript supplementation when the
      range has captured user/assistant evidence; without that fallback it
      fails permanently before AI. Missing, malformed, or unusable required
      bounded evidence still blocks the first AI call.
      PR #798 closes #792: migration v067 stores typed capture-time commit evidence, exact-range linking uses durable `session_row_id`, relative transcript workdirs are anchored to the Stop cwd, and a fail-closed non-interactive Git grammar accepts ordinary spaced/equal `--fixup` commits while rejecting editor-opening fixups, configurable output, shell expansion, redirection, globbing, process substitution, unquoted shell comments, and quiet commits as evidence sources while retaining quiet event capture and exact trailing `git status --short`. Observed success requires exit zero or Claude's explicitly named success-only `PostToolUse`, while explicit failure events override contradictory fields; Codex accepts only its wrapper status before `Final output:`, so status-like command output cannot override failure. An ambiguous, malformed, or unresolvable call is logged and skipped without erasing earlier proof; legacy spill rows without `event_id` receive stable occurrence-distinct identities, and non-Unix orphan claims use the minimum-age fallback. Late or same-identity replay evidence uses deterministic bounded `captured_git_link` work without replaying AI/rollup side effects. #795 makes automatic native-memory mirroring error-visible but non-blocking after rollup persistence; T7 remains open for #796.
- [x] Upgrade handling rejects non-terminal legacy `JobType::Summary` jobs
      instead of draining the retired AI path or converting payloads without an
      authoritative contract; migration v064 preserves terminal Summary
      history and non-summary jobs, freezes retryable failed Summary jobs before
      failure maintenance can reopen them, Stop hooks no longer enqueue new
      Summary jobs, capture-ledger failures spill instead of falling back to the
      retired writer, stale spill replay compares host/project/session before
      dropping older current-identity rows, replayed Stop captures use stable
      event IDs for retry idempotency, duplicate fixed event ID captures reuse
      the existing extraction task without reviving terminal rollup work, replay
      capture-ledger failures are preserved once by the replay layer,
      doctor/status ignore v064 upgrade rejection rows as freeze blockers and
      actionable failed jobs while keeping worker-side post-retirement Summary
      rejections visible, capture redaction preserves `cwd` and
      `transcript_path` plus its captured byte boundary for worker-side raw
      archive ingest, persisted SessionRollup side effects re-home
      summary-derived candidates, workstream upsert, native memory sync, and
      follow-up scheduling plus exact-range observed commit linking. Automatic
      native-memory filesystem failures remain visible at error level with
      exact-range identity but no longer suppress UserContextCandidate,
      Compress, or Dream follow-ups, as covered by
      `native_memory_write_failure_does_not_block_durable_rollup_followups`.
      Full T7 completion remains blocked by #796. Old-version daemon heartbeats and legacy singleton locks do not
      suppress the current Stop fallback worker, current once-launch
      heartbeats prevent overlapping fallback workers, workers claim
      extraction tasks before Compress/Dream jobs, and the worker rejects
      already-claimed Summary jobs before the retired path can run. Covered by
      `legacy_summary_upgrade_rejects_non_terminal_jobs`,
      `worker_rejects_legacy_summary_job_without_retry`,
      `summarize_hook_runs_stop_side_effects_without_summary_job`,
      `citation_failure_does_not_block_capture_payload`,
      `capture_redaction_preserves_stop_payload_paths_for_worker_side_effects`,
      `session_rollup_worker_drains_raw_archive_from_stop_payload`,
      `session_rollup_drains_every_coalesced_stop_payload`,
      `session_rollup_deduplicates_same_transcript_at_widest_stop_boundary`,
      `session_rollup_prompt_includes_only_bounded_transcript_text`,
      `session_rollup_prompt_does_not_duplicate_captured_message_text`,
      `session_rollup_missing_transcript_fails_before_metadata_only_summary`,
      `persisted_citation_evidence_keeps_long_assistant_tail`,
      `persisted_citation_evidence_survives_cross_stop_prompt_eviction`,
      `persisted_citation_evidence_covers_each_boundary_of_repeated_path`,
      `legacy_v066_citation_message_hash_stays_idempotent`,
      `total_budget_never_retains_empty_utf8_message`,
      `session_rollup_candidate_evidence_stays_with_claimed_range`,
      `session_rollup_honors_stop_transcript_snapshot_boundary`,
      `session_rollup_retries_transcript_side_effects_without_resummarizing`,
      `session_rollup_rehomes_finalize_side_effects`,
      `session_rollup_enqueues_followup_jobs_after_rollup`,
      `captured_commit_evidence_links_exact_range`,
      `captured_commit_link_retry_is_idempotent`,
      `replayed_observe_spill_preserves_commit_snapshot_when_head_moves`,
      `replay_capture_failure_is_preserved_once_by_replay_layer`,
      `replay_capture_is_idempotent_without_hook_followup_jobs`,
      `duplicate_fixed_event_id_does_not_revive_done_task`,
      `current_healthy_daemon_skips_stop_spawn`,
      `old_version_healthy_daemon_uses_stop_fallback_spawn`,
      `current_once_suppresses_spawn_with_newer_old_daemon_heartbeat`,
      `once_bypasses_lock_for_old_version_daemon_heartbeat`,
      `old_version_daemon_lock_allows_current_once_heartbeat`,
      `summarize_hook_replays_same_session_spill_for_different_project`,
      `capture_ledger_failure_blocks_followup_jobs`, and
      `legacy_surfaces_ignore_upgrade_summary_rejections_but_report_worker_rejections`,
      `upgrade_summary_rejections_are_not_actionable_but_worker_rejections_are`.
- [ ] Pending legacy migration and guarded-drop tests.
- [x] MCP/docs wording verification.
- [ ] `cargo fmt --check`, `cargo check`, focused tests, and `cargo test`
      before merge readiness.

## Rollback Plan

Do not drop current tables. If Summary retirement regresses behavior, restore
the enqueue/worker path while keeping doctor visibility. Guarded-drop
migrations are separate and can be delayed without affecting current capture.
