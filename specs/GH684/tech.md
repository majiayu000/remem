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
| Stop hook side effects | `src/summarize/summary_job/`, `src/worker.rs` | Summary path also owns other behaviors. | Preserve Compress/Dream/raw/citation/failure/candidate/native-memory side effects before removal. |

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
- [x] Stop-hook side-effect regression tests cover Compress/Dream enqueueing,
      hook-owned raw archive ingest, memory citations, failure lessons,
      summary-derived candidate finalization, and native-memory sync before
      `JobType::Summary` retirement.
- [x] Upgrade handling rejects non-terminal legacy `JobType::Summary` jobs
      instead of draining the retired AI path or converting payloads without an
      authoritative contract; migration v064 preserves terminal Summary
      history and non-summary jobs, freezes retryable failed Summary jobs before
      failure maintenance can reopen them, Stop hooks no longer enqueue new
      Summary jobs, capture-ledger failures spill instead of falling back to the
      retired writer, same-session stale spills are skipped after the current
      stop payload succeeds, doctor/status ignore explicit rejection rows as
      freeze blockers and actionable failed jobs, and the worker rejects
      already-claimed Summary jobs before the retired path can run. Covered by
      `legacy_summary_upgrade_rejects_non_terminal_jobs`,
      `worker_rejects_legacy_summary_job_without_retry`,
      `summarize_hook_runs_stop_side_effects_without_summary_job`,
      `enqueue_summary_followup_jobs_skips_legacy_summary_job`,
      `capture_ledger_failure_blocks_followup_jobs`, and
      `legacy_surfaces_ignore_explicit_summary_rejections_as_blockers`,
      `explicit_summary_rejections_are_not_actionable_job_failures`.
- [ ] Pending legacy migration and guarded-drop tests.
- [x] MCP/docs wording verification.
- [ ] `cargo fmt --check`, `cargo check`, focused tests, and `cargo test`
      before merge readiness.

## Rollback Plan

Do not drop current tables. If Summary retirement regresses behavior, restore
the enqueue/worker path while keeping doctor visibility. Guarded-drop
migrations are separate and can be delayed without affecting current capture.
