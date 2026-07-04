-- v055_session_ingest_cursors: batch session ingestion support (issue #722).
-- Per-file incremental cursors for `remem ingest-sessions`, plus a source-root
-- label on raw_messages so rows from synced/remote roots stay distinguishable.

CREATE TABLE IF NOT EXISTS ingest_cursors (
    file_path TEXT PRIMARY KEY,
    mtime_epoch INTEGER,
    size_bytes INTEGER,
    last_ingested_at INTEGER
);

ALTER TABLE raw_messages
    ADD COLUMN source_root TEXT NOT NULL DEFAULT 'local';
