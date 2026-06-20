-- v048_failure_lesson_feed_events: idempotency ledger for deterministic
-- failure-trajectory lesson distillation from raw transcripts.

CREATE TABLE IF NOT EXISTS memory_lesson_feed_events (
    id INTEGER PRIMARY KEY,
    project TEXT NOT NULL,
    session_id TEXT NOT NULL,
    source TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    lesson_memory_id INTEGER,
    outcome_kind TEXT NOT NULL CHECK (outcome_kind IN ('failure')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'saved')),
    evidence_raw_message_ids TEXT NOT NULL DEFAULT '[]',
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(project, session_id, source, source_hash),
    FOREIGN KEY(lesson_memory_id) REFERENCES memories(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_lesson_feed_events_project_recent
    ON memory_lesson_feed_events(project, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_memory_lesson_feed_events_memory
    ON memory_lesson_feed_events(lesson_memory_id);
