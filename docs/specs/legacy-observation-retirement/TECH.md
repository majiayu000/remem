# Legacy Observation Retirement Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Epic issue: #684
- Related contracts: `current-memory-contracts/`

## Existing Implementation Facts

Verified inventory (2026-07-02): static writer/reader classification of every
production reference (tests and migrations excluded), cross-checked against a
production-shaped dogfood database (schema v53, 42k memories, 8.3k sessions).

### `pending_observations` (legacy queue) — verdict: pure legacy

- Writers on the default runtime path: none. `enqueue_pending`
  (`src/db/pending/queue.rs`) has no production caller; only test bodies call
  it. The PostToolUse observe hook writes `captured_events` +
  `ObservationExtract` tasks, not this queue; the Stop hook only reads a
  count to log an "ignored N legacy pending" warning.
- Remaining writers are manual admin: `remem pending migrate-legacy`
  (`src/db/pending/admin/migration.rs`, re-records rows as `captured_events`
  and marks them `migrated`), `retry-failed` / `purge-failed`
  (`src/db/pending/admin/mutate.rs`). The claim/lease machinery
  (`src/db/pending/claim.rs`) has no production consumer.
- Readers: status/stats counters, doctor capture-liveness and queue-health,
  observability metrics, `remem pending` listings.
- Dogfood evidence: queue fully empty — ready/delayed/processing/expired/
  failed all 0.

### `observations` — verdict: reclassify-current (NOT legacy)

- Default-path writers exist in the current pipeline:
  `persist_observations` (`src/observation_extract.rs` →
  `src/db/observation.rs`) runs inside the `ObservationExtract` extraction
  task; `src/summarize/compress.rs` inserts compressed observations from the
  Stop-hook compress job. Staleness marking, dedup access bumps, and
  retention cleanup also mutate it.
- Readers are live features: MCP `get_observations(source='observation')`,
  `remem timeline`, memory-candidate promotion evidence
  (`src/memory_candidate.rs`), staleness, `remem why` git trace,
  status/stats.
- The promotion funnel counts it as a current stage:
  `captured_events -> observations -> candidates -> promoted`.
- The GH684-T8 wording fix labels MCP `get_observations(source='observation')`
  as current extracted observations. Keep that descriptor from regressing to
  "legacy observations" because that misdescribes a live intermediate store.

### `observations_fts` — verdict: current but narrow

- No Rust writer: maintained by migration-defined SQL triggers on
  `observations`.
- Single production read path: `remem timeline` anchor resolution
  (`src/cli/actions/query/timeline.rs` → `src/retrieval/search/observation.rs`
  → `src/db/query/search.rs`). It is NOT reachable from the main `search`
  MCP tool or `remem search`, which query `memories` only.
- Disposition follows `observations`.

### `session_summaries` — verdict: shared, single writer after GH684-T7

- Historical inventory found two writers reachable from the same Stop hook:
  1. Current: `persist_session_rollup` (`src/session_rollup/persist.rs`)
     via the `SessionRollup` extraction task.
  2. Legacy pre-v006: the former Summary enqueue helper
     (`src/summarize/summary_job/hook.rs`) → worker `JobType::Summary`
     (`src/worker.rs`) → `finalize_summarize`
     (`src/db/summarize/session/finalize.rs`, DELETE+INSERT).
- GH684-T7 retires the legacy enqueue path: Stop hooks record the
  `SessionRollup` extraction task and enqueue only Compress/Dream follow-up
  jobs. If capture-ledger recording fails, the hook spills the payload and
  skips follow-up jobs instead of relying on legacy Summary fallback. If the
  current stop payload succeeds, replay skips older same-session spills so the
  current capture remains authoritative. Raw archive ingest, memory-citation
  recording, and failure-lesson distillation run from the hook side-effect path
  before Compress/Dream follow-up enqueue.
- Readers are load-bearing current features: context injection sessions
  section + data-version hint, user-context recall/extraction/summary,
  timeline, `remem why`, observation-extract context, status/doctor.
- Governance mutations (`src/memory/scope_cleanup/mutate.rs`) are manual.
- The retirement target is therefore the legacy summarize job chain, not
  the table. The table stays; one of its two writers goes.
- Dogfood corroboration: the jobs queue shows 2479 failed legacy jobs, and
  AI usage attribution reports 24019 unattributed legacy calls — the legacy
  chain is not just redundant, it is actively failing and unaccounted.

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

Verified dispositions (2026-07-02 static + dogfood analysis, recorded in
Existing Implementation Facts above):

| Surface | Disposition |
|---|---|
| `pending_observations` | `retire` — no default-path writer, dogfood queue empty; drop table + claim/queue machinery after window |
| `observations` | `reclassify-current` — live intermediate of the extraction pipeline; GH684-T8 fixed the "legacy" MCP wording |
| `observations_fts` | `reclassify-current` — trigger-maintained; follows `observations` |
| `session_summaries` (table) | `keep` — load-bearing for context/timeline/user-context readers |
| legacy summary writer (former Summary enqueue helper → `JobType::Summary` → `finalize_summarize`) | `retire-summary-only` — the Summary job was the dual-writer duplicating `SessionRollup`; GH684-T7 keeps the surrounding Stop-hook Compress and Dream follow-ups while stopping new Summary job enqueue |

Remaining Phase 1 analysis before freeze decisions execute:

- Output equivalence between the two `session_summaries` writers: does
  `finalize_summarize` produce fields or quality the `SessionRollup` path
  does not (compare row shape and content on dogfood data)? If yes, port the
  delta into the rollup before removing the legacy chain.
- Confirm `pending_observations` has zero rows across other real databases,
  not only the primary dogfood one; `remem pending migrate-legacy` remains
  the escape hatch for stragglers.

## Phase 2: Doctor Visibility

- New doctor section: per-surface row count, last-write epoch, current
  state (live/frozen/...), and the planned next transition.
- After a surface is frozen, any new write raises a doctor error finding
  (and the write path itself is removed or guarded — a frozen surface with
  active writers is a bug, not a warning).
- `remem status --json` mirrors counts for scripting.

Tests: fixture DBs per state; frozen-write detection test.

## Phase 3: Writer Freeze (per verified retire set)

### Legacy summarize chain

1. Equivalence fixtures first: committed test comparing `finalize_summarize`
   output rows against `persist_session_rollup` output for the same seeded
   session; document every field-level delta.

   GH684-T2 established the delta and GH684-T3 ported the load-bearing
   row-output fields. The current fixture:
   `summary_writer_equivalence_fixture_documents_field_level_deltas`
   (`src/session_rollup/tests.rs`) locks the field contract before writer
   retirement:

   | Field group | `finalize_summarize` | `persist_session_rollup` | Reader impact |
   | --- | --- | --- | --- |
   | `completed` | parsed summary `completed` text | top-level rollup `<summary>` text | Equivalent for the fixture text |
   | `summary_text` | `NULL` | same top-level rollup summary text | Rollup-only range summary column |
   | `request` | parsed request text | semantic rollup `<structured_fields><request>` text, with `Captured event range X..Y` fallback only for old/malformed responses | GH684-T3 ports the user-facing title/source string; current readers exclude synthetic fallback rollup rows |
   | `decisions`, `learned`, `next_steps`, `preferences` | structured legacy summary fields | semantic rollup `<structured_fields>` values | GH684-T3 ports the load-bearing fields for observation extraction, user-context extraction/recall, and Claude native-memory sync |
   | `prompt_number` | `NULL` in the production Summary caller | `NULL` | Equivalent unset state in current writers |
   | `discovery_tokens` | token estimate across structured fields | token estimate across summary plus structured fields | Equivalent enough for reader accounting; not a retirement blocker |
   | `host_id`, `project_id`, `session_row_id`, `covered_*_event_id` | `NULL` | populated rollup range identity | Rollup-only range identity; GH684-T3 lets semantic rollup rows feed context/user-context while still excluding synthetic fallback range titles |
   | ownership/context columns (`source_project`, `target_project`, `owner_scope`, `owner_key`, `topic_domain`, `routing_confidence`, `routing_reason`, `context_class`, validity/expiry) | `NULL` | `NULL` | Equivalent unset state in current writers |
   | `summarize_cooldown` | updated with message hash | not updated | Legacy retry/dedup side effect; retire or replace deliberately |

   The remaining field delta is the legacy cooldown side effect, which belongs
   to Summary retirement/upgrade handling rather than row-output parity.
2. Port any load-bearing delta into the rollup path (readers must not lose
   fields they consume today). Completed by GH684-T3 for request, decisions,
   learned, next_steps, preferences, and semantic rollup reader visibility.
3. Remove only the `JobType::Summary` enqueue/worker/finalize path from the
   Stop hook path. Before deleting or renaming the shared helper, port or
   preserve its other Stop side effects: `JobType::Compress` enqueueing,
   Dream enqueueing with cooldown/dedup behavior, and profile payload
   preservation. Also preserve or explicitly replace the load-bearing
   `process_summary_job_input` side effects that currently happen before or
   around the summary AI call: raw archive ingest, failure-lesson distillation,
   memory citation/final-session summary persistence, summary-derived
   candidate finalization, and native-memory sync. Add regression tests
   proving Stop still schedules Compress and Dream and that each retained
   side effect has a new owner before Summary retirement.

   GH684-T4 locks these side effects with regression coverage before the
   Summary retirement decision in GH684-T7: Stop-hook follow-up enqueue tests
   cover Compress and Dream profile/cooldown behavior; the shared hook
   side-effect path owns raw archive ingest, memory citations before
   cooldown/summary skips, and failure-lesson distillation; finalize tests
   cover summary-derived candidate finalization;
   `process_finalized_summary_syncs_native_memory_side_effect` covers
   native-memory sync after a finalized Summary job.
4. GH684-T7 chooses rejection for in-flight legacy `JobType::Summary` jobs at
   upgrade time. Migration v064 marks non-terminal and retryable failed Summary
   jobs as failed permanent, clears lease/retry state, and records an explicit
   upgrade rejection error. The worker also rejects any already-claimed Summary
   job before it can enter the retired AI/finalize path, while doctor/status
   excludes these explicit rejection rows from freeze blockers and actionable
   failed-job counts. Stop hooks no longer enqueue new Summary jobs, and
   capture-ledger failures spill and abort follow-ups rather than falling back
   to the retired writer. When the current stop payload succeeds, older
   same-session spills are skipped during replay. This preserves terminal
   Summary history and non-summary jobs. Draining would rerun the retired AI
   path, and conversion lacks an authoritative legacy payload-to-SessionRollup
   contract.
5. Doctor: a `session_summaries` row written by anything other than the
   rollup path after freeze is an error finding.

### `pending_observations`

Readers are counters and admin listings only, so no reader migration is
needed. Freeze means: delete the dead claim/lease machinery
(`src/db/pending/claim.rs`) and the test-only `enqueue_pending` write path;
status/doctor keep reporting row counts until the drop ships.

GH684-T5 confirmed the real local databases on 2026-07-08. The default store
(`/Users/apple/.remem/remem.db`) and the dated backup stores under
`/Users/apple/Backups/remem/20260704-094200` through
`/Users/apple/Backups/remem/20260708-033004` all had zero ready, delayed,
processing, expired, and failed `pending_observations` rows. The default store
also returned zero rows from `remem pending list-failed --json`. No
`remem pending migrate-legacy` run was needed for any checked store.

GH684-T6 freezes the dead queue writer/claim surface by deleting
`enqueue_pending`, claim/lease helpers, and the legacy `PendingObservation`
claim DTO from the crate. Production builds keep the read/reporting surfaces
and admin commands (`pending migrate-legacy`, `retry-failed`, `purge-failed`,
`list-failed`) but no longer export a runtime API that can enqueue, claim,
fail, or delete claimed legacy pending rows. `retry-failed` remains only as a
migration-prep admin step: it moves failed rows back to `pending` so
`pending migrate-legacy` can replay them into `captured_events`, and CLI,
doctor, status, and README guidance point users to that follow-up migration.
Tests that need historical rows seed them through
`db::test_support::insert_legacy_pending_fixture` instead of a
production-style queue API.

### Reclassification (no freeze)

`observations` + `observations_fts` stay current. GH684-T8 updates the MCP
`get_observations` tool description and `docs/ARCHITECTURE.md` so the source is
not described as legacy.

## Phase 4: Value Migration + Drop

1. `remem pending migrate-legacy` (already exists) is the migration path for
   any non-empty `pending_observations` in the wild; extend its report to
   print migrated/skipped/valueless counts if it does not already.
2. Deprecation window: at least one minor release where doctor announces
   the upcoming drop and the release notes carry it.
3. Guarded drop migration for `pending_observations` (pre-check refuses when
   unmigrated rows exist). No drop for `observations`, `observations_fts`,
   or `session_summaries` — they stay.
4. Retire `JobType::Summary` handling and `finalize_summarize` code after
   the window; clean up historical failed legacy-job rows in the jobs table
   with an explicit `remem cleanup` action rather than a silent migration.
   Migration v064 only rejects non-terminal Summary jobs at upgrade and does
   not delete historical rows.

Tests: migration idempotency; guarded-drop refusal; post-drop schema-drift
tests (`src/migrate/schema_drift.rs`) updated in the same PR as the drop.

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

- `finalize_summarize` vs `persist_session_rollup` output equivalence: which
  fields differ, and do any current readers depend on legacy-only fields?
  (This gates the legacy-chain removal.)
- Answered by GH684-T7: in-flight `JobType::Summary` jobs are rejected at
  upgrade time by migration v064 and by a worker-side execution fence, not
  drained or converted; new Stop-hook work no longer enqueues Summary jobs.
- Should the `get_observations` MCP source keep the name
  `source='observation'` after the description fix, or is a rename worth the
  client churn?
