# Tech Spec

## Linked Issue

GH-871

## Product Spec

`specs/GH871/product.md`

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Raw CLI open path | `src/cli/actions/query/raw.rs`, `src/db/core.rs` | `raw search` and `raw sessions` call `open_db()`, which configures a read-write connection and runs migrations. `open_db_read_only()` already opens an existing database without migration. | The migration path begins a write transaction and reproduces GH-871's lock failure. |
| Batch transcript identity | `src/ingest/sessions.rs`, `src/ingest/sessions/tests.rs` | A 20-line probe prefers top-level `sessionId`/`session_id` or Codex `session_meta.payload.id`, then falls back to filename stem. The result is not persisted independently of raw rows. Unchanged cursors skip before the probe. | New metadata support cannot reconcile unchanged historical files, and operators cannot audit which identity source was used. |
| Stop-hook raw ingest | `src/session_rollup/side_effects.rs`, `src/memory/raw_archive.rs` | The worker drains the transcript under the hook/extraction task session ID. Raw uniqueness is `(source_root, project, session_id, role, content_hash)`. | Hook and batch identities can split one transcript; rekey must preserve the uniqueness contract. |
| Raw session counts | `src/memory/raw_archive.rs` | Session grouping returns only `COUNT(*)` plus optional user-message text samples. | Fixed-window comparison needs total/user/assistant counts without reading samples. |
| Transcript parsing | `src/memory/raw_transcript.rs` | Supported user/assistant text is extracted, but the parser does not expose meta/XML inclusion categories to callers. | Reconciliation must explain consumer exclusions without weakening raw capture. |
| CLI surface | `src/cli/query_types.rs`, `src/cli/actions/query/raw.rs`, `src/cli/tests_raw.rs` | `RawAction` has `Search` and `Sessions`; both support stable JSON output. | Add a scriptable `Reconcile` action while keeping existing shapes compatible. |
| Schema and drift gates | `src/migrate/types.rs`, `src/migrate/schema_drift/invariants.rs`, `src/migrate/tests_schema*.rs` | Latest schema is v070 and declared tables/indexes are checked against migration truth. | The durable identity ledger must be a declared, migration-tested schema object. |
| Parent/current contracts | `specs/GH720/tasks.md`, `docs/specs/README.md`, `README.md`, `docs/ARCHITECTURE.md` | `SP720-T4` remains pending and no current raw-session ingestion contract describes identity reconciliation. | Completion must update the parent task evidence and the user/runtime contract. |

## Proposed Design

### 1. Read-only raw query connections

Change `run_raw_search`, `run_raw_sessions`, and the new reconciliation action
to call `db::open_db_read_only()`.

The read-only open remains fail-closed for a missing database or invalid
SQLCipher key and does not create or migrate a store. `ingest-sessions` remains
the explicit write/migration surface.

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
    identity_source TEXT NOT NULL
        CHECK(identity_source IN ('transcript_metadata', 'filename_fallback')),
    status TEXT NOT NULL
        CHECK(status IN ('active', 'conflict')),
    conflict_reason TEXT,
    contract_version INTEGER NOT NULL DEFAULT 0,
    first_seen_at_epoch INTEGER NOT NULL,
    last_seen_at_epoch INTEGER NOT NULL,
    UNIQUE(source_root, transcript_path, canonical_session_id)
);

CREATE INDEX idx_raw_session_identities_fallback
    ON raw_session_identities(
        source_root, project, fallback_session_id, status
    );
CREATE INDEX idx_raw_session_identities_canonical
    ON raw_session_identities(
        source_root, project, canonical_session_id, status
    );
```

`transcript_path` is sensitive local metadata already present in
`ingest_cursors`. It remains inside the encrypted database and is never
serialized by the reconciliation surface. Storing it directly avoids an
unkeyed path hash that could be dictionary-tested.

v071 also adds these additive raw-message fields:

```sql
ALTER TABLE raw_messages ADD COLUMN event_time_source TEXT NOT NULL
    DEFAULT 'legacy_unknown'
    CHECK(event_time_source IN (
        'transcript_event', 'ingest_fallback', 'legacy_unknown'
    ));
```

New transcript rows use `transcript_event` only when `created_at_epoch` came
from a parsed record timestamp. Missing timestamps use `ingest_fallback`.
Existing rows begin as `legacy_unknown`.

Move transcript context probing before the unchanged-cursor return. Every
discovered file therefore upserts its identity ledger claim even when message
ingest is skipped. A ledger row remains `contract_version = 0` until a complete
redrain has refreshed identity and event-time provenance; version 0 bypasses an
otherwise unchanged cursor once. Successful redrain sets version 1, so later
runs recover normal incremental behavior.

### 3. Conflict-safe legacy rekey

When transcript metadata produces a canonical ID different from the filename
stem:

1. Query all identity rows, including prior conflicts, for the same
   `(source_root, project, fallback_session_id)`.
2. If any current or historical claim maps that fallback to a different
   canonical ID, or one transcript path changes its canonical claim, mark every
   row in the fallback group `conflict`. Once any group row is `conflict`, all
   future automatic rekeys for that fallback remain blocked. Retain all raw
   rows unchanged, emit an error-level diagnostic, and increment
   `identity_conflicts`.
3. Before mutation, compare every fallback/canonical collision on all
   non-identity fields: role, exact content, content hash, source, branch, cwd,
   `created_at_epoch`, and `event_time_source`. Any mismatch marks the group
   conflict and aborts the whole rekey without mutation.
4. Otherwise run one savepoint:
   - copy legacy raw rows to the canonical ID with
     `ON CONFLICT(source_root, project, session_id, role, content_hash)
     DO NOTHING`;
   - delete only the old fallback-ID rows after the copies succeed;
   - release the savepoint.
5. Record inserted/merged/rekeyed counts internally. A repeated pass finds no
   fallback rows and is a no-op.

The raw FTS insert/delete triggers maintain index consistency during the
copy/delete. The algorithm never rewrites rows whose old ID is not the
deterministic filename fallback for the discovered transcript.

### 4. Shared versioned transcript classification

Extend `raw_transcript` with a record classifier used by both archive draining
and reconciliation. Classification precedence is:

1. malformed JSON;
2. unsupported record/role;
3. missing event timestamp;
4. outside the inclusive UTC window;
5. empty supported text;
6. meta user;
7. XML/control user;
8. conversational user or assistant.

The disjoint output classes are:

- supported `user` or `assistant` text with event timestamp;
- `meta_user` when the transcript marks the user event as metadata;
- `xml_control_user` when normalized user text begins with `<`;
- `empty_text`;
- `unsupported_record`;
- `malformed_record`.

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

1. Discover transcript files with the same root and `subagents/` rules as
   `ingest-sessions`.
2. Capture each file length once and stream only complete records through that
   boundary.
3. Derive the same canonical identity and project key as ingestion.
4. Aggregate transcript-side archive-eligible role counts and the explicit
   exclusion taxonomy inside the fixed UTC window.
5. Aggregate raw rows by `(source_root, project, session_id)` for the same
   window.
6. Compare the internal keys and counts; do not serialize any key.

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
    "count_mismatch_sessions": 0,
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
    "outside_window": 0,
    "missing_event_time": 0,
    "archive_unknown_event_time": 0,
    "malformed_record": 0
  },
  "parity": true
}
```

For each mismatched shared session, positive role/count deltas accumulate into
the transcript-excess or archive-excess fields; equal-and-opposite session
deltas therefore cannot disappear behind equal global totals.

`parity` is true only when count mismatches, transcript/archive-only and
role-split excess counts, identity conflicts, malformed records, missing event
time, and archive unknown event time are all zero. Meta/XML conversational
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
unbounded private transcript history. Required custom roots fail loudly;
missing optional default roots contribute zero.

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
| P1 | raw CLI actions + read-only DB open | Unit/subprocess lock test holds `BEGIN IMMEDIATE` while both raw queries succeed; `open_db_read_only` tests remain green. |
| P2-P4 | context probe + v071 identity ledger | Claude/Codex metadata and filename-fallback fixtures; unchanged cursor still updates ledger. |
| P5-P6 | legacy rekey savepoint and conflict detection | Unique legacy rekey, canonical collision merge, idempotent rerun, ambiguous fallback refusal, rollback-on-error tests. |
| P7 | raw session aggregate/JSON | SQL fixture proves total/user/assistant counts and preserved existing fields. |
| P8-P9 | shared transcript classifier | Meta/XML/empty/unsupported/malformed fixtures prove disjoint categories while raw capture keeps supported text. |
| P10-P12 | `raw_reconcile` + CLI parsing | Exact parity, transcript-only, archive-only, mismatch, conflict, UTC boundary, empty roots, missing required root, deterministic JSON, and sensitive-sentinel absence tests. |

## Data Flow

```text
transcript discovery
  -> bounded context probe
  -> path fingerprint + identity ledger upsert
  -> conflict check
  -> optional fallback-ID raw row rekey
  -> unchanged cursor check
  -> normal bounded raw-message drain

raw reconcile (read-only)
  -> bounded transcript classifier aggregates
  +  fixed-window raw session aggregates
  -> internal identity/count comparison
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
  reparse during normal ingestion. Reconciliation is an explicit bounded
  full-window scan and streams files.
- Data integrity: copy/delete rekey can collide with canonical rows. A sticky
  all-history conflict check, exact non-identity equality preflight, savepoint,
  existing unique key, FTS triggers, and idempotency tests protect against
  loss.
- Maintenance: ingestion and reconciliation must share discovery, identity,
  timestamp, and classification helpers so the contracts cannot drift.

## Test Plan

- [ ] Unit tests: transcript classification, identity probing/fingerprinting,
      ledger upsert, conflict detection, aggregate counts, deterministic
      report serialization.
- [ ] Integration tests: unchanged-cursor identity refresh, conflict-safe
      rekey and collision merge, read-only lock contention, fixed-window
      reconciliation fixtures.
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
Rekeyed rows retain the same content, role, source, timestamps, branch, and cwd;
rollback does not attempt to reconstruct obsolete filename-fallback groupings.
Disabling the reconciliation CLI removes no data. If identity conflicts are
found, stop automatic rekey for those mappings and retain both the ledger
evidence and original raw rows for a later explicit repair.
