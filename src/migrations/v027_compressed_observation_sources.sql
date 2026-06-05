-- v027_compressed_observation_sources: deterministic source evidence for
-- compressed observations.
--
-- source_observation_id is intentionally not a foreign key. Retention may
-- delete old source observations after verifying the compressed row, while the
-- source id and hash must remain auditable.

CREATE TABLE IF NOT EXISTS compressed_observation_sources (
    id INTEGER PRIMARY KEY,
    compressed_observation_id INTEGER NOT NULL,
    source_observation_id INTEGER NOT NULL,
    source_hash TEXT NOT NULL,
    source_snapshot_json TEXT NOT NULL,
    source_created_at_epoch INTEGER NOT NULL,
    compression_session_id TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    UNIQUE(compressed_observation_id, source_observation_id),
    FOREIGN KEY(compressed_observation_id) REFERENCES observations(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_compressed_observation_sources_compressed
    ON compressed_observation_sources(compressed_observation_id);

CREATE INDEX IF NOT EXISTS idx_compressed_observation_sources_source
    ON compressed_observation_sources(source_observation_id);
