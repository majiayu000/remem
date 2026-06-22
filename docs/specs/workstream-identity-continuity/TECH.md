# Workstream Identity Continuity Technical Spec

Status: Current contract
Date: 2026-06-22

Tracking:
- Capability issue: #603

## Existing Implementation Facts

- `prompts/summary.txt` asks the summary AI to emit `<workstream>`,
  `<workstream_progress>`, `<workstream_next>`, and
  `<workstream_blockers>`.
- `src/summarize/parse.rs` parses those fields as plain strings.
- `src/summarize/summary_job/persist.rs` converts the parsed summary into a
  `ParsedWorkStream` and calls `crate::workstream::upsert_workstream`.
- `src/workstream/write.rs` calls `find_matching_workstream` before insert.
- `src/workstream/matcher.rs` matches exact title first, then simple substring
  containment among active/paused workstreams for the same owner/project.
- `workstream_sessions` links a workstream row to a `memory_session_id`, but
  `upsert_workstream` does not currently use that link before title matching.
- `build_existing_summary_context` can load a previously linked workstream for
  the same `memory_session_id`, proving the DB already has a continuity signal
  that is stronger than title text.

## Failure Mode

For the Spellbook session `019ed986-1081-7b21-92d5-99ab50923b1a`,
`summary-job` created three rows for one real task:

```text
id=1140 title=agent-workflow Skill 生命周期工作流
id=1163 title=flowguard Skill 生命周期工作流
id=1167 title=flowguard / run-guard Skill 生命周期工作流
```

The title changes were semantically related, but neither exact title matching
nor substring containment could prove identity. The session link would have
been a stronger match signal, but it was not part of the upsert matching order.

## Design

### Data Model

Add alias/history storage separate from the canonical `workstreams.title`.

Recommended migration:

```sql
ALTER TABLE workstreams ADD COLUMN identity_key TEXT;
ALTER TABLE workstreams ADD COLUMN merged_into_workstream_id INTEGER;

CREATE TABLE IF NOT EXISTS workstream_aliases (
    id INTEGER PRIMARY KEY,
    workstream_id INTEGER NOT NULL REFERENCES workstreams(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    normalized_title TEXT NOT NULL,
    first_seen_epoch INTEGER NOT NULL,
    last_seen_epoch INTEGER NOT NULL,
    UNIQUE(workstream_id, normalized_title)
);

CREATE TABLE IF NOT EXISTS workstream_alias_sources (
    id INTEGER PRIMARY KEY,
    alias_id INTEGER NOT NULL REFERENCES workstream_aliases(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    memory_session_id TEXT,
    source_workstream_id INTEGER REFERENCES workstreams(id),
    observed_title TEXT NOT NULL,
    first_seen_epoch INTEGER NOT NULL,
    last_seen_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_workstream_aliases_lookup
    ON workstream_aliases(normalized_title);

CREATE INDEX IF NOT EXISTS idx_workstream_alias_sources_alias
    ON workstream_alias_sources(alias_id);

CREATE INDEX IF NOT EXISTS idx_workstreams_identity_key
    ON workstreams(identity_key);

CREATE INDEX IF NOT EXISTS idx_workstreams_merged_into
    ON workstreams(merged_into_workstream_id);
```

`identity_key` is generated on insert and remains stable for logs, exports, and
future APIs. A deterministic local value such as
`ws_<sha256(project + first_memory_session_id + created_at_epoch)>` is enough;
it does not need to be user-authored.

`merged_into_workstream_id` keeps duplicate rows auditable while allowing query
paths to hide merged rows from active context.

Alias lookup must not duplicate ownership columns from `workstreams`. Owner or
project filters are applied by joining `workstream_aliases.workstream_id` back
to `workstreams` and filtering the canonical row. This avoids stale alias owner
copies when existing scope-cleanup governance reroutes a workstream to a new
project, owner scope, or owner key.

`workstream_alias_sources` stores per-observation provenance for an alias. The
same normalized alias may be observed in multiple summaries, sessions, or
merged duplicate rows; those source facts must not collapse into one
`memory_session_id` on the alias row. The implementation may enforce an
idempotency key for repeated source observations, but it must preserve enough
source rows to answer which session or duplicate introduced an alias, which
exact title text was observed, and when that source first and last contributed
the alias.

### Title Normalization

Add a small normalization helper in `src/workstream/`:

- trim whitespace;
- lowercase ASCII;
- collapse repeated whitespace;
- remove common punctuation separators such as `/`, `-`, `_`, `:`, and
  paired brackets for lookup only;
- keep CJK characters intact;
- do not remove broad domain words such as `skill` or `workflow` globally,
  because doing so can over-merge unrelated tasks.

Normalization is for lookup, not display.

### Upsert Matching Order

Change `upsert_workstream` to use this order:

1. `find_linked_workstream(conn, project, memory_session_id)`:
   - active/paused rows only;
   - same owner/project filters as current query paths;
   - excludes rows with `merged_into_workstream_id IS NOT NULL`;
   - safe only when the session query returns exactly one canonical candidate;
   - also requires supporting continuity evidence, such as an existing alias or
     normalized-title relation, explicit future workstream ref, or prior
     `<existing_summary>` context for the same task;
   - if the same session links multiple active workstreams, or the only link
     lacks continuity evidence, log `session_link_ambiguous` and continue to
     the next safe matcher instead of updating the prior task.
2. Explicit identity/ref match:
   - reserved for a future summary prompt contract;
   - not required for the first implementation.
3. Alias exact match:
   - lookup normalized title in `workstream_aliases`;
   - join through `workstreams` for project, owner, status, and merged-row
     filters instead of trusting copied alias ownership fields;
   - active/paused canonical rows only;
   - if multiple candidates exist, do not auto-merge; log an ambiguity warning
     and continue to the next safe path.
4. Current exact title match.
5. Conservative fuzzy fallback:
   - keep current containment behavior only as a final fallback;
   - add tests proving unrelated tasks do not merge.
6. Insert new workstream.

Every successful match must record a match reason such as:

```text
session_link
alias_exact
title_exact
title_contains
insert
```

Log the reason in the existing summary-job upsert line or an adjacent
structured log:

```text
upserted workstream id=1167 reason=session_link project=... session=...
```

### Alias Recording

After every successful insert or update:

- upsert the incoming display title into `workstream_aliases`;
- if the canonical row had a previous title, ensure that previous title also
  exists as an alias;
- update `last_seen_epoch` when an alias repeats;
- insert or update a `workstream_alias_sources` row for the source observation,
  including `memory_session_id`, source duplicate workstream ID, observed title,
  and first/last observed timestamps when available.

### Query Changes

Update active/list query paths to exclude merged duplicates:

```sql
AND merged_into_workstream_id IS NULL
```

Affected areas:

- `src/workstream/query.rs`
- `src/workstream/matcher.rs`
- `build_existing_summary_context` / `get_linked_workstream_context`, which
  must resolve merged duplicate links to their canonical row or exclude rows where
  `merged_into_workstream_id IS NOT NULL`
- context rendering paths that rely on `query_active_workstreams`
- MCP/CLI list behavior through existing query functions

### Governance Merge

Add a focused manual merge path after the core identity fix, or in the same
implementation if small:

```text
remem workstreams merge --project <path> --into <canonical_id> <duplicate_id>...
```

Merge behavior:

- validate all rows are in the same project/owner visibility scope;
- move `workstream_sessions` links with `INSERT OR IGNORE`;
- copy aliases/history to the canonical row and preserve source rows for every
  copied alias;
- set `merged_into_workstream_id` on duplicate rows;
- mark duplicate rows non-active for query purposes;
- log the merge with canonical and duplicate IDs.

MCP can expose the merge later; CLI is enough for the first repair surface.

## Migration And Backfill

Migration should be additive and safe for encrypted/local databases:

1. Add nullable `identity_key` and `merged_into_workstream_id`.
2. Create `workstream_aliases`.
3. Backfill `identity_key` for existing rows.
4. Backfill each existing `workstreams.title` as an alias with
   `source='migration'` and a matching alias source row.
5. Do not attempt automatic historical duplicate merge during migration.
   Duplicate repair is a separate explicit governance action.

## Tests

Add focused tests under `src/workstream/tests/` and
`src/summarize/summary_job/persist.rs` as appropriate:

- same `memory_session_id`, three renamed titles, one canonical row;
- same `memory_session_id`, unrelated topic switch, no automatic update of the
  prior task;
- same `memory_session_id` with multiple linked active candidates logs
  ambiguity instead of picking the first row;
- alias exact match across a later session updates the canonical row;
- alias exact match still works after scope cleanup reroutes the canonical
  workstream to a new owner/project;
- unrelated active tasks with shared broad words do not merge;
- merged duplicate rows are hidden from active query results;
- `build_existing_summary_context` excludes merged rows or resolves them to the
  canonical workstream;
- alias rows are created for old and new titles;
- alias source rows preserve each contributing session or duplicate source;
- migration convergence includes the new table/indexes/columns;
- summary persistence logs or returns enough data to assert match reason where
  practical.

Regression fixture titles:

```text
agent-workflow Skill 生命周期工作流
flowguard Skill 生命周期工作流
flowguard / run-guard Skill 生命周期工作流
```

## Rollout Plan

1. Land this spec PR with `Refs #603`.
2. Open an implementation issue linked from #603 after spec review.
3. Implement schema and same-session matching first.
4. Add alias matching and query filtering.
5. Add manual duplicate merge if scope remains small; otherwise split it into a
   follow-up implementation issue.
6. Run focused tests, then `cargo fmt --check`, `cargo check`, and broader
   `cargo test` before closing the implementation issue.

## Risks And Mitigations

| Risk | Mitigation |
|---|---|
| Over-merging unrelated tasks | Prefer session link and exact alias before fuzzy matching; keep fuzzy fallback conservative and tested. |
| Existing duplicates remain after migration | Provide explicit merge governance instead of risky automatic historical merges. |
| Alias lookup creates ambiguous candidates | Log ambiguity and avoid automatic merge when multiple canonical rows match. |
| Same session contains unrelated tasks | Require a unique linked candidate plus continuity evidence; log ambiguity rather than unconditional update. |
| Alias ownership drifts after scope cleanup | Derive project/owner filters by joining aliases to canonical workstreams. |
| Schema drift across old databases | Add migration convergence tests and backfill only nullable/additive fields. |
| LLM repeats unstable names | Treat LLM titles as display/alias input, not the sole identity source. |

## Acceptance Gates

Implementation is not complete until:

```bash
cargo fmt --check
cargo check
cargo test workstream
cargo test summarize
```

For PR submission touching migrations or shared runtime behavior, run full
`cargo test` when practical and document any skipped broader gate.
