# Session Observatory Technical Spec

Status: Current contract
Date: 2026-08-20

## Existing Implementation Facts

- Raw conversation occurrences live in `raw_messages` and are selected by the
  exact `source_root`, `project`, and `session_id` tuple.
- Identified transcripts additionally carry `transcript_identity_id` and
  `transcript_record_ordinal`.
- Tool evidence lives in append-only `captured_events`, keyed to
  `session_row_id`; capture policy intentionally omits some low-value events.
- Session-level semantic fields live in `session_summaries` as `request`,
  `completed`, `decisions`, `learned`, and `next_steps`.
- The native API already exposes `/api/v1/sessions`, `/api/v1/events`, and
  `/api/v1/stats`, but session detail currently contains metadata only.
- The local plugin app is plain HTML/CSS/JavaScript served by
  `plugins/remem/apps/remem/server.js`.

## Storage Contract

Add migration `v084_session_observatory.sql`.

### `session_turns`

One row per projected conversational turn:

```sql
CREATE TABLE session_turns (
    id INTEGER PRIMARY KEY,
    transcript_identity_id INTEGER REFERENCES raw_session_identities(id) ON DELETE RESTRICT,
    source_root TEXT NOT NULL,
    project TEXT NOT NULL,
    session_id TEXT NOT NULL,
    session_row_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
    turn_index INTEGER NOT NULL,
    user_message_id INTEGER NOT NULL REFERENCES raw_messages(id) ON DELETE CASCADE,
    understanding_message_id INTEGER REFERENCES raw_messages(id) ON DELETE SET NULL,
    result_message_id INTEGER REFERENCES raw_messages(id) ON DELETE SET NULL,
    understanding TEXT,
    understanding_source TEXT,
    actions_summary TEXT,
    actions_summary_source TEXT,
    result_status TEXT NOT NULL,
    result_summary TEXT,
    started_at_epoch INTEGER NOT NULL,
    ended_at_epoch INTEGER,
    capture_health TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    projection_version INTEGER NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(source_root, project, session_id, turn_index)
);
```

Closed values:

- `understanding_source`: `assistant_text` or null;
- `actions_summary_source`: `captured_events` or null;
- `result_status`: `answered`, `done`, `partial`, `failed`, `aborted`, or
  `unknown`;
- `capture_health`: `full`, `partial`, or `unavailable`.

Raw user and result content remains canonical in `raw_messages`. The stored
derived text is bounded and can be rebuilt.

### `session_turn_actions`

One ordered action per turn:

```sql
CREATE TABLE session_turn_actions (
    id INTEGER PRIMARY KEY,
    session_turn_id INTEGER NOT NULL REFERENCES session_turns(id) ON DELETE CASCADE,
    action_index INTEGER NOT NULL,
    kind TEXT NOT NULL,
    tool_name TEXT,
    summary TEXT NOT NULL,
    event_row_id INTEGER REFERENCES captured_events(id) ON DELETE SET NULL,
    files_json TEXT NOT NULL DEFAULT '[]',
    outcome TEXT,
    created_at_epoch INTEGER NOT NULL,
    UNIQUE(session_turn_id, action_index)
);
```

The database migration creates no historical rows. Projection is explicit and
bounded.

## Deterministic Projection

Add `src/session_activity/` with a pure projector and a transactional store.

1. Start an `IMMEDIATE` transaction, then load raw messages for one exact tuple
   inside that source snapshot. Prefer transcript ordinal, then event time and
   row ID for legacy rows. Reject tuples over 10,000 messages.
2. Start a turn at each `role = user` occurrence. Assistant messages before the
   next user occurrence belong to that turn.
3. Select the first meaningful assistant text as `understanding`; greetings or
   text shorter than the configured deterministic threshold do not qualify.
4. Select the final assistant message as the result. With no assistant result,
   use `aborted`; with no linked tool actions, use `answered`; otherwise use a
   conservative keyword classifier and allow `unknown`.
5. Link captured events only for the local source root and when the active
   transcript identity path identifies exactly one supported host (`.claude`,
   `.codex`, or `.cursor`) and an unambiguous `session_row_id` matches that host,
   the
   canonical `projects.project_path` (or an approved active project alias) and
   session identity. Other source roots fail closed instead of borrowing local
   tool evidence. Never join through basename-only `project_key`. Assign events
   by authoritative `reference_time_epoch`, falling back to event creation time
   only when reference time is absent. Equal or missing boundary timestamps
   remain unassigned and reduce capture health. Reject sessions over 20,000
   captured actions.
6. Treat an assistant message as an action-bearing result only when it occurs
   unambiguously after the last assigned action. A pre-action plan is not a
   completed result. Derive compact action summaries from redacted event
   metadata. Pass every API-visible user, assistant, action, and label field
   through the shared sensitive-text redactor before truncation. Never expose
   blob payloads or unrestricted command text through the new API.
7. Hash every projection-affecting input, including session/transcript identity,
   event-time provenance, and tool names, into a source digest.
8. In one transaction, no-op when digest and projection version match;
   otherwise upsert turns by exact tuple and turn index so continuation IDs stay
   stable, replace their child actions, and delete only stale trailing turns.
9. Session-identity rekeying invalidates every affected old and canonical
   tuple inside the same savepoint as the raw-row rewrite. A failed collision
   check rolls both operations back; a later explicit projection rebuilds the
   canonical tuple.

Projection version starts at `1`.

## API Contract

Add authenticated routes:

```text
GET /api/v1/session-activity
GET /api/v1/session-activity/{turn_id}
GET /api/v1/session-stats
POST /api/v1/session-activity/project
```

The POST route accepts one exact tuple and projects it explicitly. The normal
UI list may request bounded lazy projection for visible sessions through the
plugin server; API reads never silently trigger unbounded work.

Session listing uses an opaque raw-row cursor bound to its project filter and a
fixed scan budget proportional to the requested result limit. A page may be
sparse or empty while `has_more=true`; clients continue with `next_cursor`.
Dedicated recency and exact-tuple indexes bound both the candidate scan and its
latest-occurrence probes without grouping the full archive. Per-session role
counts inspect at most 10,001 messages, return counts capped at 10,000 with
`message_counts_truncated=true`, and omit `first_epoch` when the true first
message lies outside that bound. Turn listing
returns `has_more` and `next_before_id`. At most 100 actions are returned per turn, with
`actions_truncated=true` when more exist. The app follows at most five 200-turn
pages and labels the bounded 1,000-turn view when more remain. Statistics
default to 30 days and reject windows wider than 366 days.

The turn response includes evidence IDs and capture health. It does not expose
raw filesystem transcript paths, content blobs, unrestricted tool inputs, API
tokens, or secrets.

## Plugin App Plan

Extend the existing local app server rather than create a second server.

- Add backend methods for activity list/detail/stats/projection.
- Add local `/api/session-activity`, `/api/session-turn`, and
  `/api/session-stats` routes.
- Keep loopback and local POST-origin protections.
- Expose widget-accessible host tools for every activity route; embedded Apps
  SDK mode must not fall back to iframe-relative HTTP requests.
- Re-run idempotent exact-tuple projection whenever a session is selected, and
  discard stale async responses when the selection changes.
- Rework the widget into a persistent application shell with Overview,
  Sessions, Memory, and System navigation.
- Use a dense editorial/technical visual direction: warm paper background,
  ink typography, restrained signal colors, and evidence-forward turn cards.

## Backfill

Historical projection must be explicit and bounded. A follow-up CLI may expose:

```text
remem sessions rebuild --latest 100
remem sessions rebuild --project <project>
remem sessions rebuild --since <epoch>
```

The first implementation may project exact tuples through the API/app while
the CLI remains a follow-up, but it must not claim that migration backfilled
history.

## Tests

Required focused coverage:

- migration creates tables, constraints, indexes, and cascade behavior;
- turn splitting preserves repeated identical occurrences by ordinal;
- missing understanding/result stays explicit;
- ambiguous or absent event linkage cannot report full capture;
- unchanged projection is idempotent;
- changed projection atomically replaces the exact tuple;
- API auth, pagination, filters, detail, stats, and structured errors;
- plugin server routes and UI tool descriptors;
- desktop and mobile visual inspection.

Recommended commands:

```bash
cargo fmt --check
cargo check
cargo test session_activity
cargo test api_public
node --test plugins/remem/apps/remem/server.test.js
cargo test
```
