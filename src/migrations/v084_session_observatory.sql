-- v084_session_observatory: rebuildable turn-level activity above raw evidence.

CREATE TABLE session_turns (
    id INTEGER PRIMARY KEY,
    transcript_identity_id INTEGER REFERENCES raw_session_identities(id) ON DELETE RESTRICT,
    source_root TEXT NOT NULL,
    project TEXT NOT NULL,
    session_id TEXT NOT NULL,
    session_row_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
    turn_index INTEGER NOT NULL CHECK(turn_index >= 1),
    user_message_id INTEGER NOT NULL REFERENCES raw_messages(id) ON DELETE CASCADE,
    understanding_message_id INTEGER REFERENCES raw_messages(id) ON DELETE SET NULL,
    result_message_id INTEGER REFERENCES raw_messages(id) ON DELETE SET NULL,
    understanding TEXT,
    understanding_source TEXT CHECK(
        understanding_source IS NULL OR understanding_source = 'assistant_text'
    ),
    actions_summary TEXT,
    actions_summary_source TEXT CHECK(
        actions_summary_source IS NULL OR actions_summary_source = 'captured_events'
    ),
    result_status TEXT NOT NULL CHECK(
        result_status IN ('answered', 'done', 'partial', 'failed', 'aborted', 'unknown')
    ),
    result_summary TEXT,
    started_at_epoch INTEGER NOT NULL,
    ended_at_epoch INTEGER,
    capture_health TEXT NOT NULL CHECK(
        capture_health IN ('full', 'partial', 'unavailable')
    ),
    source_digest TEXT NOT NULL,
    projection_version INTEGER NOT NULL CHECK(projection_version >= 1),
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(source_root, project, session_id, turn_index)
);

CREATE TABLE session_turn_actions (
    id INTEGER PRIMARY KEY,
    session_turn_id INTEGER NOT NULL REFERENCES session_turns(id) ON DELETE CASCADE,
    action_index INTEGER NOT NULL CHECK(action_index >= 1),
    kind TEXT NOT NULL CHECK(kind IN ('read', 'edit', 'create', 'delete', 'run', 'search', 'external', 'other')),
    tool_name TEXT,
    summary TEXT NOT NULL,
    event_row_id INTEGER REFERENCES captured_events(id) ON DELETE SET NULL,
    files_json TEXT NOT NULL DEFAULT '[]',
    outcome TEXT CHECK(outcome IS NULL OR outcome IN ('succeeded', 'failed', 'unknown')),
    created_at_epoch INTEGER NOT NULL,
    UNIQUE(session_turn_id, action_index)
);

CREATE INDEX idx_session_turns_recent
    ON session_turns(started_at_epoch DESC, id DESC);
CREATE INDEX idx_session_turns_project_recent
    ON session_turns(project, started_at_epoch DESC, id DESC);
CREATE INDEX idx_session_turns_session
    ON session_turns(source_root, project, session_id, turn_index);
CREATE INDEX idx_session_turns_identity
    ON session_turns(transcript_identity_id, turn_index)
    WHERE transcript_identity_id IS NOT NULL;
CREATE INDEX idx_session_turn_actions_turn
    ON session_turn_actions(session_turn_id, action_index);
CREATE INDEX idx_session_turn_actions_event
    ON session_turn_actions(event_row_id)
    WHERE event_row_id IS NOT NULL;

CREATE INDEX idx_raw_messages_activity_tuple_recent
    ON raw_messages(source_root, project, session_id, created_at_epoch DESC, id DESC);
CREATE INDEX idx_raw_messages_activity_recent
    ON raw_messages(created_at_epoch DESC, id DESC, source_root, project, session_id);
CREATE INDEX idx_raw_messages_project_activity_recent
    ON raw_messages(project, created_at_epoch DESC, id DESC, source_root, session_id);
