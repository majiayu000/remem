DROP INDEX IF EXISTS idx_memory_embeddings_model;
DROP INDEX IF EXISTS idx_memory_embeddings_profile_memory_id;

CREATE TABLE IF NOT EXISTS memory_embeddings_v058 (
    memory_id INTEGER NOT NULL,
    embedding BLOB NOT NULL,
    dimensions INTEGER NOT NULL,
    model TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    PRIMARY KEY(memory_id, model, dimensions),
    FOREIGN KEY(memory_id) REFERENCES memories(id) ON DELETE CASCADE
);

INSERT OR REPLACE INTO memory_embeddings_v058
    (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
SELECT memory_id, embedding, dimensions, model, content_hash, updated_at_epoch
FROM memory_embeddings;

DROP TABLE memory_embeddings;
ALTER TABLE memory_embeddings_v058 RENAME TO memory_embeddings;

CREATE INDEX IF NOT EXISTS idx_memory_embeddings_model
    ON memory_embeddings(model, updated_at_epoch);

CREATE INDEX IF NOT EXISTS idx_memory_embeddings_profile_memory_id
    ON memory_embeddings(model, dimensions, memory_id);
