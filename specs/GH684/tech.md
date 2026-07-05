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
| `observations` | `src/observation_extract.rs`, `src/db/observation.rs`, MCP/context/timeline readers | Live current intermediate. | Fix misleading legacy wording; do not retire. |
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
7. Fix MCP and architecture docs that describe live observations as legacy.
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
- [ ] `finalize_summarize` versus `persist_session_rollup` equivalence tests.
- [ ] Stop-hook side-effect regression tests.
- [ ] Pending legacy migration and guarded-drop tests.
- [ ] MCP/docs wording verification.
- [ ] `cargo fmt --check`, `cargo check`, focused tests, and `cargo test`
      before merge readiness.

## Rollback Plan

Do not drop current tables. If Summary retirement regresses behavior, restore
the enqueue/worker path while keeping doctor visibility. Guarded-drop
migrations are separate and can be delayed without affecting current capture.
