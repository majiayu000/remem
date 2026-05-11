-- v005_memories_fts_active_filter: keep memories_fts in sync with active status only.
--
-- The original memories_au trigger re-inserted every row on any UPDATE, leaving
-- superseded ('stale') and archived rows in the FTS index. Search code paths that
-- forgot to join on status='active' returned superseded content. Issue #70.
--
-- Rebuild the FTS table from active rows only instead of issuing per-row FTS5
-- delete commands. Some legacy databases may already be missing a non-active
-- row from FTS, and deleting a row that is not indexed can make FTS5 report
-- "database disk image is malformed".

DROP TRIGGER IF EXISTS memories_ai;
DROP TRIGGER IF EXISTS memories_ad;
DROP TRIGGER IF EXISTS memories_au;
DROP TABLE IF EXISTS memories_fts;

CREATE VIRTUAL TABLE memories_fts USING fts5(
    title, content,
    content='memories',
    content_rowid='id',
    tokenize='trigram'
);

CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, title, content)
    SELECT new.id, new.title, new.content WHERE new.status = 'active';
END;

CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, content)
    VALUES ('delete', old.id, old.title, old.content);
END;

CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, content)
    VALUES ('delete', old.id, old.title, old.content);
    INSERT INTO memories_fts(rowid, title, content)
    SELECT new.id, new.title, new.content WHERE new.status = 'active';
END;

INSERT INTO memories_fts(rowid, title, content)
SELECT id, title, content FROM memories WHERE status = 'active';
