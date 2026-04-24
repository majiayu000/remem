-- v002_raw_messages: introduce raw archive layer (spec: SPEC-raw-archive-vs-curated-memory-2026-04-22)

CREATE TABLE IF NOT EXISTS raw_messages (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    project TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    source TEXT NOT NULL,
    branch TEXT,
    cwd TEXT,
    created_at_epoch INTEGER NOT NULL,
    UNIQUE(project, role, content_hash)
);

CREATE INDEX IF NOT EXISTS idx_raw_messages_project_created
    ON raw_messages(project, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_raw_messages_session
    ON raw_messages(session_id, created_at_epoch);

CREATE VIRTUAL TABLE IF NOT EXISTS raw_messages_fts USING fts5(
    content,
    content='raw_messages',
    content_rowid='id',
    tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS raw_messages_ai AFTER INSERT ON raw_messages BEGIN
    INSERT INTO raw_messages_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS raw_messages_ad AFTER DELETE ON raw_messages BEGIN
    INSERT INTO raw_messages_fts(raw_messages_fts, rowid, content)
    VALUES ('delete', old.id, old.content);
END;

CREATE TRIGGER IF NOT EXISTS raw_messages_au AFTER UPDATE ON raw_messages BEGIN
    INSERT INTO raw_messages_fts(raw_messages_fts, rowid, content)
    VALUES ('delete', old.id, old.content);
    INSERT INTO raw_messages_fts(rowid, content) VALUES (new.id, new.content);
END;
