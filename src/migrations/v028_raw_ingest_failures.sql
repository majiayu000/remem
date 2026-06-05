-- v028_raw_ingest_failures: durable health records for raw archive ingest.

CREATE TABLE IF NOT EXISTS raw_ingest_failures (
    id INTEGER PRIMARY KEY,
    project TEXT NOT NULL,
    session_id TEXT NOT NULL,
    source TEXT NOT NULL,
    transcript_path TEXT,
    error_kind TEXT NOT NULL,
    error_message TEXT NOT NULL,
    inserted INTEGER NOT NULL DEFAULT 0,
    duplicates INTEGER NOT NULL DEFAULT 0,
    parse_errors INTEGER NOT NULL DEFAULT 0,
    insert_errors INTEGER NOT NULL DEFAULT 0,
    created_at_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_raw_ingest_failures_project_recent
    ON raw_ingest_failures(project, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_raw_ingest_failures_session
    ON raw_ingest_failures(session_id, created_at_epoch DESC);
