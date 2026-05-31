-- v020_memory_fts_all_status: index ALL memory rows in memories_fts regardless of status.
--
-- Issue #236. The v012 triggers (inherited from v005, issue #70) only inserted rows
-- WHERE status = 'active'. But fts.rs queries with `m.status IN ('active','stale','archived')`
-- when include_inactive=true, so stale/archived rows never entered memories_fts and the
-- JOIN dropped them — include_inactive searches on the bm25 path silently returned empty.
--
-- Fix: rebuild the triggers WITHOUT the status filter so every row is indexed. Visibility
-- is now enforced purely by the post-JOIN `m.status` predicate in fts.rs
-- (memory_status_filter_sql), which is the single source of truth for what callers see.

DROP TRIGGER IF EXISTS memories_ai;
DROP TRIGGER IF EXISTS memories_ad;
DROP TRIGGER IF EXISTS memories_au;
DROP TABLE IF EXISTS memories_fts;

CREATE VIRTUAL TABLE memories_fts USING fts5(
    title, content, search_context,
    content='memories',
    content_rowid='id',
    tokenize='trigram'
);

CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, title, content, search_context)
    VALUES (new.id, new.title, new.content, COALESCE(new.search_context, ''));
END;

CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, content, search_context)
    VALUES ('delete', old.id, old.title, old.content, COALESCE(old.search_context, ''));
END;

CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, content, search_context)
    VALUES ('delete', old.id, old.title, old.content, COALESCE(old.search_context, ''));
    INSERT INTO memories_fts(rowid, title, content, search_context)
    VALUES (new.id, new.title, new.content, COALESCE(new.search_context, ''));
END;

INSERT INTO memories_fts(rowid, title, content, search_context)
SELECT id, title, content, COALESCE(search_context, '')
FROM memories;
