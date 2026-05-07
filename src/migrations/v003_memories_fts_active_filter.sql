-- v003_memories_fts_active_filter: keep memories_fts in sync with active status only.
--
-- The original memories_au trigger re-inserted every row on any UPDATE, leaving
-- superseded ('stale') and archived rows in the FTS index. Search code paths that
-- forgot to join on status='active' returned superseded content. Issue #70.
--
-- This migration is defensive against legacy schemas that pre-date the FTS layer:
-- it ensures memories_fts and its three triggers exist in the corrected form,
-- and backfills the index by removing non-active rows.

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    title, content,
    content='memories',
    content_rowid='id',
    tokenize='trigram'
);

DROP TRIGGER IF EXISTS memories_ai;
CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, title, content)
    SELECT new.id, new.title, new.content WHERE new.status = 'active';
END;

DROP TRIGGER IF EXISTS memories_ad;
CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, content)
    VALUES ('delete', old.id, old.title, old.content);
END;

DROP TRIGGER IF EXISTS memories_au;
CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, content)
    VALUES ('delete', old.id, old.title, old.content);
    INSERT INTO memories_fts(rowid, title, content)
    SELECT new.id, new.title, new.content WHERE new.status = 'active';
END;

-- Backfill: drop existing non-active rows that the old trigger left in the FTS index.
INSERT INTO memories_fts(memories_fts, rowid, title, content)
SELECT 'delete', id, title, content FROM memories WHERE status != 'active';
