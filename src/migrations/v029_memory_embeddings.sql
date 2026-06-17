CREATE TABLE IF NOT EXISTS memory_embeddings (
    memory_id INTEGER PRIMARY KEY,
    embedding BLOB NOT NULL,
    dimensions INTEGER NOT NULL,
    model TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    FOREIGN KEY(memory_id) REFERENCES memories(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_memory_embeddings_model
    ON memory_embeddings(model, updated_at_epoch);

CREATE INDEX IF NOT EXISTS idx_memory_embeddings_profile_memory_id
    ON memory_embeddings(model, dimensions, memory_id);
