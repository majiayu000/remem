# Raw Session Ingestion Technical Contract

Status: Current contract
Refs: GH-871, GH-720

## Persistence

Migration v071 adds `raw_session_identities` keyed by
`(source_root, transcript_path)` and a separate
`raw_session_identity_claims` history. Ledger rows retain canonical and
fallback IDs, current and legacy project aliases, sticky conflict state,
captured mtime/size, event range, missing-time count, explicit event-index
status, and contract version.

`raw_messages` stores `event_time_source`, `transcript_identity_id`, and the
complete JSONL record ordinal. A partial unique index makes occurrence replay
idempotent while allowing identical content at different ordinals. Legacy
matching rows are claimed in place; later repeated occurrences insert
separately. File drains roll back on read, parse, or insert failure.

## Identity Flow

1. Discover JSONL files with the shared scan-root/subagents rules.
2. Probe metadata ID, filename fallback, project aliases, and captured tuple.
3. Persist all path/claim rows and resolve complete fallback groups; any prior
   group conflict is inherited by later path identities.
4. For active identities, use one fallback-group savepoint to stream immutable
   boundaries, merge exact unmatched legacy aliases before canonical rekey,
   upgrade legacy provenance/occurrence rows, rewrite and deduplicate evidence
   references, and advance ledgers/cursors only after the entire group
   succeeds.
5. Stop performs the shared identity/project probe inside its captured byte
   boundary and persists the claim, but leaves complete-set legacy convergence
   to the next batch pass.

## Read and Reconcile Flow

`open_db_read_only_current()` combines SQLite read-only flags with the same
current-schema and drift validation used by no-migrate hook opens.

Reconciliation:

1. rejects an inverted window and missing required roots;
2. captures a file descriptor, byte boundary, mtime, and size;
3. requires an exact ledger tuple before content parsing; files skipped by an
   ingest `--since` mtime bound retain an explicit `since_indexed` event index
   and can be omitted when wholly outside the requested event window, while
   pending version-0 rows from failed ingestion remain stale, selected active
   identities require a version-1 cursor, and current sticky conflicts remain
   version 0;
4. selects event-range intersections plus missing-time-bearing files;
5. classifies complete records with the shared parser and window precedence;
6. compares internal `(identity, ordinal, role, content_hash)` multisets for
   requested source-root labels only, and groups transcript-event archive rows
   without a discoverable identity by their private
   `(source_root, project, session_id)` key as archive-only;
7. counts only conflicts reached through selected identities; and
8. serializes the fixed aggregate report.

Post-capture appends are outside the immutable byte boundary. Timestamped
out-of-window records are discarded before category counters. Non-transcript
event-time rows are counted separately and excluded from strict window parity.

## Verification

Focused coverage lives in migration identity tests, ingest/session identity
tests, Stop rollup tests, raw archive/classifier tests, read-only current-schema
tests, CLI parse tests, and `memory::raw_reconcile` tests. Repository completion
also requires formatting, check, full tests, clippy, plugin-version sync,
workflow checks, preflight, PR CI, independent review, and PR gate evidence.
