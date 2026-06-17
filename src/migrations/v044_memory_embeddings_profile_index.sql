CREATE INDEX IF NOT EXISTS idx_memory_embeddings_profile_memory_id
    ON memory_embeddings(model, dimensions, memory_id);
