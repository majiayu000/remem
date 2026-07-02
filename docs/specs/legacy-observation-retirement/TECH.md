# Legacy Observation Retirement Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Epic issue: #684
- Related contracts: `current-memory-contracts/`

## Existing Implementation Facts

Grep-based starter inventory (2026-07-02, to be completed and re-verified in
Phase 1):

### `pending_observations` (legacy queue)

- Writers: `src/observe/hook.rs`, `src/summarize/summary_job/hook.rs`
  (hook-side enqueue paths), `src/db/pending/queue.rs`.
- Queue/claim machinery: `src/db/pending/{claim,query,queue}.rs`,
  `src/db/pending/admin/{migration,mutate,query}.rs` (includes
  `migrate_legacy_pending` and truncation shims).
- Readers/reporting: `src/cli/actions/query/status.rs`,
  `src/db/query/observability/*`, `src/db/query/stats.rs`,
  `src/doctor/{capture_liveness,database,health_action}.rs`.
- Schema: `v001_baseline.sql`, touched by `v003`, `v006`.

### `observations` + `observations_fts` (legacy derived store)

- FTS/search readers: `src/db/query/search.rs`,
  `src/retrieval/search/observation.rs`.
- MCP exposure: `get_observations` with `source='observation'` documented as
  "legacy observations" (`src/mcp/server/context_tools.rs`).
- Writer paths to be confirmed in Phase 1 (legacy flush machinery).

### `session_summaries` (shared surface — NOT purely legacy)

- Current-pipeline writers: `src/session_rollup/persist.rs`,
  `src/summarize/summary_job/persist.rs`, finalize in
  `src/db/summarize/session/finalize.rs`.
- Readers: `src/context/query.rs`, `src/context/claude_memory/runtime.rs`,
  `src/context/injection_gate/data_version_hint.rs`,
  `src/timeline/{summary,detail}.rs`, `src/git_trace.rs`,
  `src/user_context/{extraction/source,recall/sources,summary}.rs`,
  `src/observation_extract.rs`, `src/worker.rs`, stats/status/doctor.
- Schema: `v001`, `v007`, `v019`, `v035`.

The `session_summaries` finding matters: the current session-rollup pipeline
still writes this table, so it is a live shared surface, not a retirement
candidate by default. Its disposition needs the Phase 1 analysis before any
freeze decision.

## Design Rules

- Convergence, not rewrite: every phase must leave the capture-ledger path
  byte-identical in behavior.
- State machine per surface: `live -> frozen -> migrated -> removed`, no
  skipped states, each transition observable via doctor.
- Reads move before writes die: a consumer switches to a ledger-backed
  source only with committed equivalence fixtures (same query, old vs new
  source, compared output).
- Drop migrations are guarded: they refuse to run when the doctor pre-check
  finds unmigrated rows above zero (excluding rows explicitly classified
  valueless in the spec decision).
- All classification decisions land in this file via spec-update PRs, so the
  decision history is reviewable.

## Phase 1: Inventory + Decision (spec-only)

Deliverable: replace the starter inventory above with a verified table:

| Surface | Ref | Kind (writer/reader) | Trigger path | Disposition |
|---|---|---|---|---|

Disposition values: `retire` (migrate then drop), `freeze` (read-only until
removal date), `reclassify-current` (not legacy; document why), `keep`
(deliberate audit surface).

Decision inputs required per surface:

- last-write timestamps from production-shaped databases (dogfood evidence);
- whether any unique value exists that `captured_events`/`raw_messages`/
  promoted memories do not represent;
- consumer list and replacement source for each.

Expected (to validate, not assume): `pending_observations` and
`observations(+fts)` -> `retire`; `session_summaries` ->
`reclassify-current` or a narrower split (legacy columns/rows vs rollup
output).

## Phase 2: Doctor Visibility

- New doctor section: per-surface row count, last-write epoch, current
  state (live/frozen/...), and the planned next transition.
- After a surface is frozen, any new write raises a doctor error finding
  (and the write path itself is removed or guarded — a frozen surface with
  active writers is a bug, not a warning).
- `remem status --json` mirrors counts for scripting.

Tests: fixture DBs per state; frozen-write detection test.

## Phase 3: Reader Migration + Freeze

Per surface classified `retire`:

1. Build ledger-backed replacements for each reader (search over
   `captured_events`/memories instead of `observations_fts`, etc.).
2. Equivalence fixtures: committed test comparing old-source and new-source
   output on seeded data; acceptable deltas documented in the fixture.
3. Flip readers to the new source behind the same public API; MCP
   `source='observation'` remains as a deprecated, documented audit
   passthrough.
4. Remove/guard writers; surface enters `frozen`.

## Phase 4: Value Migration + Drop

1. `remem migrate legacy-observations` (idempotent): moves rows with unique
   value into the ledger (as `captured_events` with a legacy retention
   class) or promotes them where the Phase 1 decision says so; reports
   migrated/skipped/valueless counts.
2. Deprecation window: at least one minor release where doctor announces
   the upcoming drop and the release notes carry it.
3. Drop migration per table with the guarded pre-check; `observations_fts`
   drops with its base table; MCP legacy source parameter removed in the
   same release, with the MCP tool description updated.

Tests: migration idempotency; guarded-drop refusal; post-drop schema-drift
tests (`src/migrate/schema_drift.rs`) updated in the same PR as each drop.

## Compatibility Notes

- Downgrade is not supported across a drop migration; the schema-version
  gate already refuses old binaries on new schemas, which covers this.
- Encrypted databases: migration commands go through the normal open path;
  no special casing.
- `db/pending/admin/migration.rs` (`migrate_legacy_pending`) becomes the
  seed for the Phase 4 command rather than a parallel mechanism.

## Verification

```bash
cargo fmt --check
cargo check
cargo test
```

Plus per-phase: equivalence fixtures (Phase 3), migration idempotency +
guarded-drop tests (Phase 4), and a dogfood-database dry run recorded in the
epic before each drop ships.

## Open Questions

- Does `observations` hold rows predating `raw_messages` whose text exists
  nowhere else (true unique value), and how many?
- Is a `legacy` retention class in `captured_events` the right landing zone,
  or should valuable legacy rows become `pending_review` candidates instead?
- Timeline output contract: do we pin current rendering with snapshot tests
  before switching its source, or accept documented deltas?
