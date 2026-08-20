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
    transcript_identity_id INTEGER REFERENCES raw_session_identities(id),
    source_root TEXT NOT NULL,
    project TEXT NOT NULL,
    session_id TEXT NOT NULL,
    session_row_id INTEGER REFERENCES sessions(id),
    turn_index INTEGER NOT NULL,
    user_message_id INTEGER NOT NULL REFERENCES raw_messages(id),
    understanding_message_id INTEGER REFERENCES raw_messages(id),
    result_message_id INTEGER REFERENCES raw_messages(id),
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
    event_row_id INTEGER REFERENCES captured_events(id),
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

1. Load raw messages for one exact tuple. Prefer transcript ordinal, then event
   time and row ID for legacy rows.
2. Start a turn at each `role = user` occurrence. Assistant messages before the
   next user occurrence belong to that turn.
3. Select the first meaningful assistant text as `understanding`; greetings or
   text shorter than the configured deterministic threshold do not qualify.
4. Select the final assistant message as the result. With no assistant result,
   use `aborted`; with no linked tool actions, use `answered`; otherwise use a
   conservative keyword classifier and allow `unknown`.
5. Link captured events only when an unambiguous `session_row_id` matches the
   project/session identity. Assign events to timestamp windows. Equal or
   missing timestamps reduce capture health instead of being guessed.
6. Derive compact action summaries from redacted event metadata. Never expose
   blob payloads or unrestricted command text through the new API.
7. Hash the ordered raw-message IDs/hashes and linked event IDs/hashes into a
   source digest.
8. In one transaction, no-op when digest and projection version match;
   otherwise delete and replace only this exact tuple's projected rows.

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

List and stats parameters support bounded `project`, `since`, `until`, cursor,
and limit filters. Defaults and maximum limits follow existing read-resource
patterns.

The turn response includes evidence IDs and capture health. It does not expose
raw filesystem transcript paths, content blobs, unrestricted tool inputs, API
tokens, or secrets.

## Plugin App Plan

Extend the existing local app server rather than create a second server.

- Add backend methods for activity list/detail/stats/projection.
- Add local `/api/session-activity`, `/api/session-turn`, and
  `/api/session-stats` routes.
- Keep loopback and local POST-origin protections.
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

