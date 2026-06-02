-- v022_topic_segments: Topic Loom intermediate layer for topic continuity.
--
-- Session rollup emits coherent per-topic segments between captured events and
-- final promoted memories. Memories remain the durable promoted product.
--
-- Event ranges can overlap when parallel work interleaves. evidence_event_ids
-- is the authoritative event link; covered_from/to_event_id are derived min/max
-- for ordering and range queries.

CREATE TABLE IF NOT EXISTS topic_segments (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER NOT NULL REFERENCES sessions(id),
    project TEXT NOT NULL,
    topic_key TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    status TEXT NOT NULL,
    segment_index INTEGER NOT NULL,
    covered_from_event_id INTEGER NOT NULL,
    covered_to_event_id INTEGER NOT NULL,
    evidence_event_ids TEXT NOT NULL,
    files TEXT,
    confidence REAL NOT NULL DEFAULT 0.75,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_topic_segments_trace
    ON topic_segments(project_id, topic_key, covered_from_event_id);

CREATE INDEX IF NOT EXISTS idx_topic_segments_session
    ON topic_segments(session_row_id, topic_key);
