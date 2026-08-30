# Raw Session Ingestion Technical Contract

Status: Current contract
Refs: GH-871, GH-720, GH-825

## Persistence

Migration v071 adds `raw_session_identities` keyed by
`(source_root, transcript_path)` and a separate
`raw_session_identity_claims` history. Ledger rows retain canonical and
fallback IDs, current and legacy project aliases, sticky conflict state,
captured mtime/size, event range, missing-time count, explicit event-index
status, and contract version.

`raw_messages` stores `event_time_source`, `transcript_identity_id`, and the
complete JSONL record ordinal. A partial unique index makes occurrence replay
idempotent while allowing identical content at different ordinals. Historical
rows whose `transcript_identity_id` is null have no trusted host provenance;
matching one raises `RawIdentityConflict` before claim, rekey, evidence rewrite,
or projection invalidation. File drains roll back on read, parse, or insert
failure.

Migration v091 adds a checked nullable `host` column to transcript identities.
Current ingestion writes an explicit host at the trusted probe boundary.
Additional batch/reconcile roots require the breaking `HOST:LABEL=PATH` syntax
and accept only `claude-code` or `codex-cli`; Cursor uses the approved Stop
snapshot path. Default roots carry their host explicitly, and batch probing
never infers the host from path components. The migration backfills only unambiguous legacy path shapes;
unknown and multi-runtime shapes remain null and make host-bound session reads
fail closed. Raw-session grouping uses `(source_root, host, project,
session_id)`, while raw-message pagination binds the cursor to the same host.

GH-825 adds a versioned Cursor parser for Stop rollup and capture-health
evidence without extending the raw archive schema. It does not project Cursor
snapshot messages into identified `raw_messages`; complete Cursor transcripts
therefore are not part of the Refine raw-session source-of-truth contract.

`raw reconcile` rejects Cursor filesystem roots at its entry boundary. This
prevents Cursor records from reaching `raw_transcript::classify_transcript_line`
and prevents unsupported-only Cursor input from producing empty successful
parity. Approved Cursor snapshots remain on the shared GH-825 Stop parser and
projection path. Reconciliation remains read-only and aggregate-only.

The additive session-level `capture_health` selector first chooses the current
source Stop by the immutable source event
`(captured_events.created_at_epoch, captured_events.id)`, then applies
`full > degraded > blank` only to outcomes for that Stop. The source row ID is
only the same-epoch capture-order tie-breaker; outcome IDs, insertion time, and
replay never advance the selected Stop. Thus an earlier full cannot mask a
later degraded or blank Stop. `raw sessions
--json` unions the raw-message aggregation with authoritative Cursor outcomes
before project/time filtering and final ordering. An outcome without a
raw-backed tuple emits one virtual `source_root: "cursor-outcome"` row using
the source Stop epoch for both bounds, zero message/role counts, and empty
samples; the virtual locator is output-only and never enters `raw_messages` or
identity tables. `raw messages --json` accepts that exact virtual locator only
to return an empty page with the same capture-health envelope. Raw-backed
selectors, message ordering, and snapshot pagination otherwise remain
unchanged. `cursor-outcome` is a reserved scan-root label:
`ScanRoot::parse` rejects user input with that label before database work, and
the ingest entry point also rejects a required `ScanRoot` constructed without
the parser before discovery or identity mutation. Default roots remain valid.
A persisted raw-message or identity row using the reserved label is an
explicit collision and makes the reader fail rather than merging it with the
virtual row. The stable JSON shape is:

```json
{
  "capture_health": {
    "fidelity": "full|degraded|blank",
    "status": "<approved Stop status>",
    "reason_code": "<stable code or null>",
    "stop_key": "<opaque redacted key>"
  }
}
```

`full` requires a null reason. `blank` requires
`reason_code: "no_usable_evidence"`, no authoritative summary ID/range, and an
empty outcome content; it does not synthesize a summary. Non-Cursor sessions
and sessions without a terminal Cursor outcome serialize
`capture_health: null`. A corrupt outcome, incompatible schema, or database
read failure propagates as a non-zero CLI error and cannot silently serialize
as null or an empty success. Older blank/degraded summaries remain audit
history and never override a higher-priority outcome for the same Stop; late
approved payload evidence upgrades blank to degraded within that Stop
regardless of outcome row time. Doctor retains all Stop outcomes for history
and counts even though session projections expose only the selected source
Stop.

## Identity Flow

1. Parse an exact host for every root, then discover JSONL files with the shared
   scan-root/subagents rules.
2. Probe metadata ID, filename fallback, project aliases, and captured tuple
   using that trusted root host.
3. Persist all path/claim rows and resolve complete fallback groups; any prior
   group conflict is inherited by later path identities.
4. For active identities, use one fallback-group savepoint to stream immutable
   boundaries and advance ledgers/cursors only after the entire group succeeds.
   An identity-null historical transcript row is unresolved provenance and
   fails before any legacy claim/rekey or dependent evidence mutation. A
   `--since`-excluded active identity may receive a bounded event index, but a
   Phase-A conflict is a failed file rather than a skipped file.
5. Claude/Codex Stop ingestion performs the shared identity/project probe
   inside its captured byte boundary and persists the claim, but leaves
   complete-set legacy convergence to the next batch pass. Cursor Stop
   snapshots remain rollup/capture-health evidence rather than raw sessions.

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
also requires those focused regressions, `cargo fmt --check`, `cargo check`,
`cargo test`, `cargo clippy --all-targets -- -D warnings`,
`python3 scripts/ci/check_plugin_version_sync.py`,
`python3 scripts/ci/check_pr_preflight.py --fast`, ordinary PR CI, and
independent review.
GH-825 additionally requires parser and rollup fixtures for approved Cursor
snapshot evidence; it does not claim raw-occurrence insertion coverage.
`memory::raw_reconcile` coverage includes fail-closed Cursor filesystem-root
rejection before capture or parity calculation.
CLI coverage also freezes `capture_health` for `raw sessions --json` and
`raw messages --json`: raw-backed plus outcome-only union/dedup, reserved
locator parser/ingest rejection, persisted-collision failure and virtual
empty-page behavior, project/time filtering, stable ordering,
full/degraded/blank/null fixtures, per-Stop `full > degraded > blank` ordering,
earlier-full/later-blank, earlier-blank/later-full, same-epoch source-Stop ID
tie and replay stability, blank no-summary invariants, and corrupt-outcome
non-zero failure.
