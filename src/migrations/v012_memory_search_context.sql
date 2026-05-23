-- v012_memory_search_context: add rebuildable retrieval context for memory FTS.
--
-- `content` remains the canonical memory body. `search_context` stores bounded,
-- deterministic hints generated from stored metadata so lexical search can match
-- files, command labels, and lifecycle terms without mutating the memory text.

ALTER TABLE memories ADD COLUMN search_context TEXT;

UPDATE memories
SET search_context = trim(
    'type: ' || COALESCE(memory_type, '') || char(10) ||
    CASE
        WHEN topic_key IS NOT NULL AND topic_key <> ''
        THEN 'topic: ' || replace(replace(topic_key, '-', ' '), '_', ' ') || char(10)
        ELSE ''
    END ||
    CASE
        WHEN files IS NOT NULL AND files <> ''
        THEN 'files: ' || files || char(10)
        ELSE ''
    END
)
WHERE search_context IS NULL;

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
    SELECT new.id, new.title, new.content, COALESCE(new.search_context, '')
    WHERE new.status = 'active';
END;

CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, content, search_context)
    SELECT 'delete', old.id, old.title, old.content, COALESCE(old.search_context, '')
    WHERE old.status = 'active';
END;

CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, content, search_context)
    SELECT 'delete', old.id, old.title, old.content, COALESCE(old.search_context, '')
    WHERE old.status = 'active';
    INSERT INTO memories_fts(rowid, title, content, search_context)
    SELECT new.id, new.title, new.content, COALESCE(new.search_context, '')
    WHERE new.status = 'active';
END;

INSERT INTO memories_fts(rowid, title, content, search_context)
SELECT id, title, content, COALESCE(search_context, '')
FROM memories
WHERE status = 'active';
