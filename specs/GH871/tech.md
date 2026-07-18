# Tech Spec

## Linked Issue

GH-871

## Product Spec

`specs/GH871/product.md`

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Raw CLI open path | `src/cli/actions/query/raw.rs`, `src/db/core.rs` | `raw search` and `raw sessions` call `open_db()`, which configures a read-write connection and runs migrations. `open_db_read_only()` already opens an existing database without migration, while `open_db_no_migrate()` contains the current-schema fail-closed check. | The migration path begins a write transaction and reproduces GH-871's lock failure; the replacement must retain schema validation without writes. |
| Batch transcript identity | `src/ingest/sessions.rs`, `src/ingest/sessions/tests.rs` | A 20-line probe prefers top-level `sessionId`/`session_id` or Codex `session_meta.payload.id`, then falls back to filename stem. The result is not persisted independently of raw rows. Unchanged cursors skip before the probe. | New metadata support cannot reconcile unchanged historical files, and operators cannot audit which identity source was used. |
| Stop-hook raw ingest | `src/session_rollup/side_effects.rs`, `src/memory/raw_archive.rs` | The worker drains the transcript under the hook/extraction task session ID. Raw uniqueness is `(source_root, project, session_id, role, content_hash)`. | Hook and batch identities can split one transcript; rekey must preserve the uniqueness contract. |
| Raw session counts | `src/memory/raw_archive.rs` | Session grouping returns only `COUNT(*)` plus optional user-message text samples. | Fixed-window comparison needs total/user/assistant counts without reading samples. |
| Transcript parsing | `src/memory/raw_transcript.rs` | Supported user/assistant text is extracted, but the parser does not expose meta/XML inclusion categories to callers. | Reconciliation must explain consumer exclusions without weakening raw capture. |
| CLI surface | `src/cli/query_types.rs`, `src/cli/actions/query/raw.rs`, `src/cli/tests_raw.rs` | `RawAction` has `Search` and `Sessions`; both support stable JSON output. | Add a scriptable `Reconcile` action while keeping existing shapes compatible. |
| Schema and drift gates | `src/migrate/types.rs`, `src/migrate/schema_drift/invariants.rs`, `src/migrate/tests_schema*.rs` | Latest schema is v070 and declared tables/indexes are checked against migration truth. | The durable identity ledger must be a declared, migration-tested schema object. |
| Parent/current contracts | `specs/GH720/tasks.md`, `docs/specs/README.md`, `README.md`, `docs/ARCHITECTURE.md` | `SP720-T4` remains pending and no current raw-session ingestion contract describes identity reconciliation. | Completion must update the parent task evidence and the user/runtime contract. |

## Proposed Design

### 1. Read-only raw query connections

Add `db::open_db_read_only_current()`: it opens with the same read-only SQLite
flags as `open_db_read_only()`, then runs the non-mutating schema-current and
schema-drift checks shared with `open_db_no_migrate()`. Change
`run_raw_search`, `run_raw_sessions`, and the new reconciliation action to use
this wrapper.

The read-only open remains fail-closed for a missing database, invalid
SQLCipher key, stale schema, or schema drift and does not create or migrate a
store. A stale schema returns the same actionable "run a migration-capable
remem command" diagnostic as the existing no-migrate path.
`ingest-sessions` remains the explicit write/migration surface.

### 2. Durable transcript identity ledger

Add migration `v071_raw_session_identity.sql` with:

```sql
CREATE TABLE raw_session_identities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_root TEXT NOT NULL,
    transcript_path TEXT NOT NULL,
    fallback_session_id TEXT NOT NULL,
    canonical_session_id TEXT NOT NULL,
    project TEXT NOT NULL,
    legacy_project TEXT NOT NULL,
    status TEXT NOT NULL
        CHECK(status IN ('active', 'conflict')),
    conflict_reason TEXT,
    contract_version INTEGER NOT NULL DEFAULT 0,
    observed_mtime_ns INTEGER NOT NULL,
    observed_size_bytes INTEGER NOT NULL,
    first_event_epoch INTEGER,
    last_event_epoch INTEGER,
    missing_event_time_count INTEGER NOT NULL DEFAULT 0,
    first_seen_at_epoch INTEGER NOT NULL,
    last_seen_at_epoch INTEGER NOT NULL,
    UNIQUE(source_root, transcript_path)
);

CREATE TABLE raw_session_identity_claims (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    transcript_identity_id INTEGER NOT NULL
        REFERENCES raw_session_identities(id) ON DELETE RESTRICT,
    claimed_session_id TEXT NOT NULL,
    identity_source TEXT NOT NULL
        CHECK(identity_source IN ('transcript_metadata', 'filename_fallback')),
    first_seen_at_epoch INTEGER NOT NULL,
    last_seen_at_epoch INTEGER NOT NULL,
    UNIQUE(transcript_identity_id, claimed_session_id, identity_source)
);

CREATE INDEX idx_raw_session_identities_fallback
    ON raw_session_identities(
        source_root, legacy_project, fallback_session_id, status
    );
CREATE INDEX idx_raw_session_identities_canonical
    ON raw_session_identities(
        source_root, project, canonical_session_id, status
    );
CREATE INDEX idx_raw_session_identity_claims_session
    ON raw_session_identity_claims(claimed_session_id, identity_source);
```

`transcript_path` is sensitive local metadata already present in
`ingest_cursors`. It remains inside the encrypted database and is never
serialized by the reconciliation surface. Storing it directly avoids an
unkeyed path hash that could be dictionary-tested.

One `raw_session_identities.id` belongs permanently to one
`(source_root, transcript_path)` and never changes when a filename fallback is
promoted to metadata canonical identity. `canonical_session_id` is the latest
non-conflicting active value; `raw_session_identity_claims` retains every
fallback and metadata claim used to derive it. `project` is the current
canonical `project_from_cwd` value, while `legacy_project` is the historical
transcript-directory slug used by the GH720-era importer. The ledger keeps both
so v071 can locate legacy raw rows without guessing or changing the public
canonical grouping. The observed cursor tuple, inclusive event range, and
missing-event-time count form a privacy-local transcript index used to avoid
parsing unrelated history without losing timestamp-less records during
reconciliation.

v071 rebuilds `raw_messages` once to add event-time provenance and stable
transcript occurrence identity:

```sql
event_time_source TEXT NOT NULL DEFAULT 'legacy_unknown'
    CHECK(event_time_source IN (
        'transcript_event', 'ingest_fallback', 'legacy_unknown'
    )),
transcript_identity_id INTEGER
    REFERENCES raw_session_identities(id) ON DELETE RESTRICT,
transcript_record_ordinal INTEGER,
CHECK(
    (transcript_identity_id IS NULL AND transcript_record_ordinal IS NULL)
    OR
    (transcript_identity_id IS NOT NULL AND transcript_record_ordinal IS NOT NULL)
);

CREATE UNIQUE INDEX idx_raw_messages_transcript_occurrence
    ON raw_messages(
        transcript_identity_id, transcript_record_ordinal
    )
    WHERE transcript_identity_id IS NOT NULL;

CREATE UNIQUE INDEX idx_raw_messages_non_transcript_content
    ON raw_messages(source_root, project, session_id, role, content_hash)
    WHERE transcript_identity_id IS NULL;
```

New transcript rows use `transcript_event` only when `created_at_epoch` came
from a parsed record timestamp. Missing timestamps use `ingest_fallback`.
The zero-based complete-JSONL record ordinal is captured inside the same
immutable file boundary used by Stop, batch, and reconcile, and the path-stable
ledger row identifies its transcript. Replaying one occurrence is idempotent,
while two identical turns at different ordinals remain two rows. Promotion
updates the ledger's active canonical value and raw project/session keys but
never changes `transcript_identity_id`. Existing rows begin as `legacy_unknown`
with null occurrence fields; the version-1 redrain assigns the lowest matching
ordinal to an existing legacy row and inserts every additional repeated
occurrence, transactionally restoring lossless cardinality.

Move transcript context probing before the unchanged-cursor return. Every
discovered file therefore upserts its identity ledger claim even when message
ingest is skipped. A ledger row remains `contract_version = 0` until a complete
redrain has refreshed identity/event-time provenance and recorded the observed
cursor tuple plus event range; version 0 bypasses an otherwise unchanged cursor
once. Successful redrain sets version 1, so later runs recover normal
incremental behavior.

Batch ingestion is explicitly two-phase. Phase A discovers every transcript in
all requested roots, probes identity/project/event-range metadata, and upserts
every ledger claim without changing raw rows or cursors. It then computes
fallback-group conflicts across the complete persisted claim set. Only after
Phase A commits with no new ambiguity does Phase B refresh/rekey/drain files.
Filesystem order can therefore never decide which canonical claim wins.

Stop-hook capture calls the same bounded identity/project probe for its
transcript path and upserts the ledger claim before draining. It writes new
messages under the metadata-derived canonical ID. The Stop path never performs
legacy fallback rekey because it has not run the complete Phase-A discovery;
the next batch pass performs that convergence safely.

### 3. Conflict-safe legacy rekey

When transcript metadata produces a canonical ID different from the filename
stem:

1. Query all path-stable identity rows and their complete claim history,
   including prior conflicts, for the same
   `(source_root, fallback_session_id)` and both project aliases. Query fallback
   raw rows under both `legacy_project` and current `project`, and canonical
   rows under `project`; never assume the aliases are equal or that a
   filename-fallback row used only the legacy slug.
2. Treat the expected filename-fallback self-claim
   (`claim.identity_source = 'filename_fallback'` and
   `claim.claimed_session_id = identity.fallback_session_id`) as promotable
   evidence, not a competing authoritative ID. If any row is already
   `conflict`, two metadata
   claims map the fallback to different canonical IDs, or one transcript path
   changes its metadata-derived canonical claim, mark every path-stable row in
   the fallback group `conflict`. Once any group row is `conflict`, all future automatic
   rekeys for that fallback remain blocked. Retain all raw rows unchanged, emit
   an error-level diagnostic, and increment `identity_conflicts`.
3. Before mutation, compare every fallback/canonical collision on stable
   occurrence identity and provenance: path-stable transcript ledger ID, record ordinal,
   role, exact content, content hash, source, `created_at_epoch`, and
   `event_time_source`. Hook-time `branch` and `cwd` are volatile provenance and
   are not conflict keys. When stable fields match, retain the canonical
   transcript-derived row's branch/cwd; when they do not, mark the group
   conflict and abort the whole rekey without mutation.
4. Otherwise run one savepoint:
   - update a non-colliding legacy row's project/session identity in place so
     its `raw_messages.id` remains stable;
   - for a collision, build an explicit old-row-ID to surviving-canonical-ID
     map, rewrite and deduplicate every persisted raw-message evidence
     reference (including lesson/feed JSON ID arrays) through one shared
     helper, then delete only the now-unreferenced duplicate row;
   - assert that no persisted reference targets a row scheduled for deletion;
   - release the savepoint.
5. Record inserted/merged/rekeyed counts internally. A repeated pass finds no
   fallback rows and is a no-op.

The raw FTS update/delete triggers maintain index consistency. A schema-aware
test enumerates every persisted raw-message reference store and fails when a
new store is added without the rewrite helper. The algorithm never rewrites
rows whose old ID is not the deterministic filename fallback for the
discovered transcript.

The version-1 redrain also refreshes occurrence/event-time provenance on
existing legacy rows instead of relying on ignored inserts:

- exact content/hash/role matches may update `event_time_source` from
  `legacy_unknown` to `transcript_event` and set `created_at_epoch` to the
  parsed transcript timestamp;
- a re-observed record with no source timestamp updates
  `legacy_unknown` to `ingest_fallback` but retains its stored ingestion epoch;
- a row already marked `transcript_event` must match the parsed event epoch or
  the identity group becomes a sticky conflict;
- an unclaimed legacy content match is assigned the path-stable ledger ID and
  lowest matching
  transcript ordinal, and later identical occurrences insert distinct rows;
- no other stable non-identity field changes during provenance refresh.

Provenance refresh, collision equality preflight, in-place/reference-safe rekey, and cursor
advance execute in one transaction. `contract_version` advances from 0 to 1
only after all row updates and the cursor succeed. Any error rolls back the
updates and leaves version 0 so the next run retries the complete upgrade.

### 4. Shared versioned transcript classification

Extend `raw_transcript` with a record classifier used by both archive draining
and reconciliation. Classification precedence is:

1. malformed JSON;
2. when a timestamp is parseable, outside the inclusive UTC window (discarded
   before all classification counters);
3. unsupported record/role;
4. missing event timestamp;
5. empty supported text;
6. meta user;
7. XML/control user;
8. conversational user or assistant.

The disjoint output classes are:

- supported `user` or `assistant` text with event timestamp;
- `meta_user` when the transcript marks the user event as metadata;
- `xml_control_user` when normalized user text begins with `<`;
- `missing_event_time`;
- `empty_text`;
- `unsupported_record`;
- `malformed_record`.

Records with a parsed timestamp outside the requested range are not members of
the reconciliation universe and do not increment an `outside_window` counter.

Raw draining preserves current behavior for supported non-empty text, including
meta/XML user text. Reconciliation reports `meta_user` and
`xml_control_user` as conversational-count exclusions while still including
their persisted rows in archive-parity totals.

For raw persistence, `created_at_epoch` is transcript event time when parsed and
`event_time_source = 'transcript_event'`; otherwise it is ingestion time with
`event_time_source = 'ingest_fallback'`. Strict fixed-window parity includes
only transcript events and raw rows with `transcript_event`. Missing transcript
time, `ingest_fallback`, and `legacy_unknown` raw rows are counted separately
and make `parity = false` until a successful version-1 redrain supplies event
time or proves that no source timestamp exists.

### 5. Session count contract

Change the session aggregate SQL to compute:

```sql
COUNT(*) AS message_count,
SUM(CASE WHEN role = 'user' THEN 1 ELSE 0 END) AS user_message_count,
SUM(CASE WHEN role = 'assistant' THEN 1 ELSE 0 END)
    AS assistant_message_count
```

Add the two fields to `RawSessionSummary`; preserve all existing field names,
grouping, ordering, and sample behavior.

### 6. Privacy-safe reconciliation module

Add `src/memory/raw_reconcile.rs` rather than expanding
`raw_archive.rs` beyond its size guard.

Inputs:

- inclusive `since_epoch` and `until_epoch` (both required);
- default transcript roots plus repeatable `--root label=path`;
- an existing read-only database connection.

Processing:

1. Reject `since_epoch > until_epoch`. Discover transcript paths with the same
   root and `subagents/` rules as `ingest-sessions`, but do not parse them yet.
2. Open each discovered path, capture its file-descriptor mtime/size tuple and
   immutable read boundary once, then require that captured tuple to match its
   ledger entry exactly. Active identities must also match a version-1 cursor.
   Sticky conflict identities remain contract version 0 because ingestion
   deliberately refuses mutation; when their ledger tuple is current, stream
   their captured boundary directly so the report can count window-relevant
   conflicts. Reject other stale, missing, or extra required-root entries with
   an actionable `run remem ingest-sessions` diagnostic. Appends after the
   captured boundary are outside this run.
3. Select files whose inclusive first/last event bounds can intersect the
   requested window, plus every file whose ledger
   `missing_event_time_count > 0`. Stream only complete records through each
   selected file's already-validated captured boundary.
4. Derive the same canonical identity and project key as ingestion.
5. Aggregate transcript-side archive-eligible message identities
   `(transcript identity, record ordinal, role, content_hash)` and the explicit
   exclusion taxonomy inside the fixed UTC window.
6. Aggregate all raw message identities for the same window. Rows with a
   transcript identity compare by
   `(transcript_identity_id, transcript_record_ordinal, role, content_hash)`.
   Historical/manual rows without a transcript identity group internally by
   `(source_root, project, session_id)` and remain archive-only; their private
   grouping keys are never serialized. Rows with ingest-fallback or
   legacy-unknown event time increment their explicit exclusion counter.
7. Query distinct persisted `status = 'conflict'` fallback groups only through
   the selected/window-relevant ledger identities (including selected
   missing-time files); count them even when transcript/raw totals otherwise
   match. Conflicts belonging only to out-of-window ledger entries do not affect
   this report.
8. Compare internal per-message multisets before collapsing them to aggregate
   counts; do not serialize any key or hash.

Stable JSON shape:

```json
{
  "policy_version": 1,
  "since_epoch": 0,
  "until_epoch": 0,
  "transcript": {
    "sessions": 0,
    "messages": 0,
    "user_messages": 0,
    "assistant_messages": 0
  },
  "archive": {
    "sessions": 0,
    "messages": 0,
    "user_messages": 0,
    "assistant_messages": 0
  },
  "comparison": {
    "exact_sessions": 0,
    "message_mismatch_sessions": 0,
    "transcript_only_sessions": 0,
    "transcript_only_messages": 0,
    "archive_only_sessions": 0,
    "archive_only_messages": 0,
    "transcript_excess_messages": 0,
    "transcript_excess_user_messages": 0,
    "transcript_excess_assistant_messages": 0,
    "archive_excess_messages": 0,
    "archive_excess_user_messages": 0,
    "archive_excess_assistant_messages": 0,
    "identity_conflicts": 0
  },
  "intentional_exclusions": {
    "meta_user": 0,
    "xml_control_user": 0,
    "empty_text": 0,
    "unsupported_record": 0,
    "missing_event_time": 0,
    "archive_ingest_fallback_event_time": 0,
    "archive_unknown_event_time": 0,
    "malformed_record": 0
  },
  "parity": true
}
```

For each mismatched shared session, multiset differences of internal
`(transcript identity, record ordinal, role, content_hash)` identities
accumulate into the transcript-excess or archive-excess fields. Equal role
counts with a missing message and an unrelated extra message therefore produce
both excess counters instead of an exact session, and repeated identical turns
remain distinct. Ordinals, ledger IDs, and hashes remain process-local
comparison keys and are never serialized.

`parity` is true only when message mismatches, transcript/archive-only and
role-split excess counts, identity conflicts, malformed records, missing event
time, archive ingest-fallback event time, and archive legacy-unknown event time
are all zero. A persisted identity conflict makes the command return a
non-zero status after emitting the aggregate report. Meta/XML conversational
exclusions do not make archive parity false when their supported text remains
present in raw totals.

Human output renders the same aggregate fields. It must not add examples,
paths, projects, IDs, hashes, or message previews.

### 7. CLI and contract wiring

Add:

```text
remem raw reconcile \
  --since <epoch|ISO8601|YYYY-MM-DD> \
  --until <epoch|ISO8601|YYYY-MM-DD> \
  [--root label=path]... \
  [--json]
```

Both bounds are required so a script cannot accidentally scan and compare an
unbounded private transcript history, and `since > until` is rejected.
Required custom roots fail loudly; missing optional default roots contribute
zero. A stale transcript ledger also fails before content parsing so the
operator must run `ingest-sessions` to refresh the bounded local index.

After implementation evidence:

- mark GH720 `SP720-T4` complete in `specs/GH720/tasks.md` with the sanitized
  fixed-window artifact and PR reference;
- add `docs/specs/raw-session-ingestion/PRODUCT.md` and `TECH.md` as the
  current contract and index them in `docs/specs/README.md`;
- update README raw-session and JSON-field documentation;
- update architecture identity and raw-query data flow.

No GitHub comment, label, or state change is made to GH720 in this bounded
tranche.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 | raw CLI actions + validated read-only DB open | Unit/subprocess lock test holds `BEGIN IMMEDIATE` while both raw queries succeed; stale/drifted schema tests prove fail-closed diagnostics without writes. |
| P2-P4 | shared Stop/batch context probe + v071 identity ledger | Claude/Codex metadata and filename-fallback fixtures; unchanged cursor still updates ledger; Stop writes canonical rows. |
| P5-P6 | two-phase discovery + legacy-project rekey savepoint and conflict detection | Filesystem-order-independent ambiguity refusal, GH720 directory-slug lookup, unique in-place rekey, collision merge with evidence-reference rewrite, idempotent rerun, rollback-on-error tests. |
| P7 | raw session aggregate/JSON | SQL fixture proves total/user/assistant counts and preserved existing fields. |
| P8-P9 | shared transcript classifier | Meta/XML/empty/unsupported/malformed fixtures prove disjoint categories while raw capture keeps supported text. |
| P10-P12 | `raw_reconcile` + CLI parsing | Exact message-identity parity, equal-count substitution, transcript-only, archive-only, persisted conflict, ingest-fallback/legacy event-time counts, inverted/UTC bounds, stale index, empty roots, missing required root, deterministic JSON, bounded-file reads, and sensitive-sentinel absence tests. |

## Data Flow

```text
transcript discovery
  -> phase A: all-path bounded identity/project/event-range probes
  -> encrypted-local transcript path + identity ledger upsert
  -> complete-set conflict check
  -> phase B:
  -> versioned event-time provenance refresh
  -> optional legacy-project/fallback-ID in-place raw row rekey
  -> collision evidence-reference rewrite
  -> unchanged cursor check
  -> normal bounded raw-message drain

raw reconcile (read-only)
  -> current-ledger/cursor validation
  -> event-range-selected transcript classifier aggregates
  +  fixed-window raw message identities
  +  persisted identity-conflict groups
  -> internal message-identity multiset comparison
  -> privacy-safe aggregate JSON/human report
```

There are no network or LLM calls. SQLite remains the only persistence layer.

## Alternatives Considered

- Only replace `open_db()` with `open_db_read_only()`: rejected because it
  fixes the reproduced lock but leaves GH-871 identity/count acceptance
  unmeasurable.
- Use the recap script's first-eight-character session ID: rejected because it
  is a display abbreviation, can collide, and is not a durable identity.
- Rewrite every historical raw session from filename heuristics in migration
  SQL: rejected because the database does not contain enough transcript-path
  metadata to make that safe.
- Store message text or example IDs in the reconciliation artifact: rejected
  because aggregate evidence is sufficient and the issue explicitly requires
  privacy-safe output.
- Drop meta/XML rows to imitate recap counts: rejected because raw capture is
  the lossless archive and consumer exclusions belong in explicit metrics.
- Add the identity fields directly to `ingest_cursors`: rejected because a
  failed or partial message drain must not advance the success cursor, while
  identity discovery still needs durable conflict evidence.

## Risks

- Security: transcript paths and content are sensitive. The new public report
  is aggregate-only; the ledger stores the local path only inside the encrypted
  database alongside identity/project values already present there. Plaintext
  development databases retain their existing explicit opt-in warning and are
  outside the report's confidentiality guarantee.
- Compatibility: session JSON gains additive fields. Rekey changes historical
  session grouping only when a deterministic metadata ID proves the old
  filename fallback.
- Performance: unchanged files receive a bounded context probe, but no full
  reparse during normal ingestion after the v1 upgrade. Reconciliation validates
  the captured file tuple and parses only files whose indexed event ranges
  intersect the requested window or contain timestamp-less records; it never
  opens unrelated historical files or counts out-of-window records.
- Data integrity: rekey can collide with canonical rows. Complete-set sticky
  conflict checks, stable-field equality preflight, in-place updates,
  evidence-reference rewrites, savepoints, the occurrence/content partial
  unique indexes, FTS triggers, and idempotency tests protect against loss.
- Maintenance: ingestion and reconciliation must share discovery, identity,
  timestamp, and classification helpers so the contracts cannot drift.

## Test Plan

- [ ] Unit tests: transcript classification, identity probing/path-ledger
      persistence, promotable fallback self-claims, sticky metadata conflicts,
      historical project aliases, evidence-reference enumeration/rewrite,
      aggregate counts, deterministic report serialization.
- [ ] Integration tests: unchanged-cursor identity refresh, conflict-safe
      two-phase rekey and collision merge, Stop/batch canonical identity,
      transactional legacy event-time provenance upgrade with rollback/version
      retry, validated read-only lock contention, equal-count message
      substitution, repeated-identical-turn preservation, window-scoped
      persisted-conflict reporting, inverted bounds, stale pre-capture index
      refusal, post-capture append snapshot preservation, missing-time-only file
      selection, bounded candidate-file reads, and fixed-window fixtures.
- [ ] Migration tests: v071 name/order, upgrade preservation, declared
      table/index/check constraints, rerun, rollback, and schema-drift
      detection.
- [ ] Deterministic checks:
      `cargo fmt --check`,
      `cargo check`,
      focused GH-871 tests,
      `cargo test`,
      `python3 checks/check_workflow.py --repo .`,
      and
      `python3 checks/check_workflow.py --repo . --spec-dir specs/GH871`.
- [ ] Manual verification:
      `remem ingest-sessions --json`, then
      `remem raw reconcile --since 1783653658 --until 1784258459 --json`;
      preserve only the sanitized aggregate artifact.

## Rollback Plan

The read-only query change can be reverted independently. The v071 identity
table is additive and can remain unused if identity reconciliation is disabled.
Rekeyed rows retain the same content, role, source, timestamps, and stable raw
row/evidence identity; canonical transcript provenance may replace volatile
hook-time branch/cwd values during a collision merge. Rollback does not attempt
to reconstruct obsolete filename-fallback groupings.
Disabling the reconciliation CLI removes no data. If identity conflicts are
found, stop automatic rekey for those mappings and retain both the ledger
evidence and original raw rows for a later explicit repair.
