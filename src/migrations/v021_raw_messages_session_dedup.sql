-- v021_raw_messages_session_dedup: add session_id to the raw_messages dedup key.
--
-- Issue #237. v002 declared UNIQUE(project, role, content_hash) — missing session_id.
-- raw_archive.rs uses ON CONFLICT(project, role, content_hash) DO NOTHING, so the same
-- text drained from two different sessions was globally deduped and the second session's
-- turn was silently dropped. Raw archive must preserve every turn per session.
--
-- Fix: (1) collapse pre-existing duplicates down to the earliest row per
-- (project, session_id, role, content_hash); (2) rebuild raw_messages with
-- UNIQUE(project, session_id, role, content_hash). SQLite cannot alter a UNIQUE
-- constraint in place, so the table is recreated and rows are copied over.

-- (1) Delete duplicates that would survive global dedup but are distinct under the
-- session-scoped key. Keep the earliest (smallest id) row in each session-scoped group.
DELETE FROM raw_messages
WHERE id NOT IN (
    SELECT MIN(id)
    FROM raw_messages
    GROUP BY project, session_id, role, content_hash
);

-- (2) Rebuild the table with the corrected UNIQUE constraint.
-- Drop FTS triggers first so the bulk copy does not double-fire them; the FTS table
-- is rebuilt from the final row set below.
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
    UNIQUE(project, session_id, role, content_hash)
);

INSERT INTO raw_messages
    (id, session_id, project, role, content, content_hash, source, branch, cwd, created_at_epoch)
SELECT id, session_id, project, role, content, content_hash, source, branch, cwd, created_at_epoch
FROM raw_messages_old;

DROP TABLE raw_messages_old;

CREATE INDEX IF NOT EXISTS idx_raw_messages_project_created
    ON raw_messages(project, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_raw_messages_session
    ON raw_messages(session_id, created_at_epoch);

-- Recreate FTS triggers (same bodies as v002).
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

-- Rebuild the FTS index from the final row set.
INSERT INTO raw_messages_fts(raw_messages_fts) VALUES ('delete-all');
INSERT INTO raw_messages_fts(rowid, content)
SELECT id, content FROM raw_messages;
