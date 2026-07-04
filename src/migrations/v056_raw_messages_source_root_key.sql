-- v056_raw_messages_source_root_key: keep batch-ingested roots distinct.
--
-- v055 added raw_messages.source_root, but the durable dedup key still ignored
-- it. Remote/custom roots with the same project/session/role/content tuple
-- were silently collapsed into the first root. Rebuild the table so
-- source_root participates in the UNIQUE key and add a created-at-leading
-- index for all-project raw session windows.

DELETE FROM raw_messages
WHERE id NOT IN (
    SELECT MIN(id)
    FROM raw_messages
    GROUP BY source_root, project, session_id, role, content_hash
);

DROP TRIGGER IF EXISTS raw_messages_ai;
DROP TRIGGER IF EXISTS raw_messages_ad;
DROP TRIGGER IF EXISTS raw_messages_au;

ALTER TABLE raw_messages RENAME TO raw_messages_old;

CREATE TABLE raw_messages (
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
    source_root TEXT NOT NULL DEFAULT 'local',
    UNIQUE(source_root, project, session_id, role, content_hash)
);

INSERT INTO raw_messages
    (id, session_id, project, role, content, content_hash, source, branch, cwd,
     created_at_epoch, source_root)
SELECT id, session_id, project, role, content, content_hash, source, branch, cwd,
       created_at_epoch, COALESCE(NULLIF(source_root, ''), 'local')
FROM raw_messages_old;

DROP TABLE raw_messages_old;

CREATE INDEX IF NOT EXISTS idx_raw_messages_project_created
    ON raw_messages(project, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_raw_messages_session
    ON raw_messages(session_id, created_at_epoch);

CREATE INDEX IF NOT EXISTS idx_raw_messages_created_source_project_session
    ON raw_messages(created_at_epoch DESC, source_root, project, session_id);

CREATE TRIGGER raw_messages_ai AFTER INSERT ON raw_messages BEGIN
    INSERT INTO raw_messages_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER raw_messages_ad AFTER DELETE ON raw_messages BEGIN
    INSERT INTO raw_messages_fts(raw_messages_fts, rowid, content)
    VALUES ('delete', old.id, old.content);
END;

CREATE TRIGGER raw_messages_au AFTER UPDATE ON raw_messages BEGIN
    INSERT INTO raw_messages_fts(raw_messages_fts, rowid, content)
    VALUES ('delete', old.id, old.content);
    INSERT INTO raw_messages_fts(rowid, content) VALUES (new.id, new.content);
END;

INSERT INTO raw_messages_fts(raw_messages_fts) VALUES ('delete-all');
INSERT INTO raw_messages_fts(rowid, content)
SELECT id, content FROM raw_messages;
