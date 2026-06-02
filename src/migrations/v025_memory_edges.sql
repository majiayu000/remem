-- v025_memory_edges: durable memory provenance and replacement relations.
--
-- memory_operation_log explains why a write happened. memory_edges records
-- durable relationships between the old/current memory rows that write touched.

CREATE TABLE IF NOT EXISTS memory_edges (
    id INTEGER PRIMARY KEY,
    edge_type TEXT NOT NULL,
    from_memory_id INTEGER,
    to_memory_id INTEGER,
    state_key_id INTEGER,
    source_candidate_id INTEGER,
    evidence_event_ids TEXT,
    source_operation_id INTEGER,
    confidence REAL,
    reason TEXT,
    created_at_epoch INTEGER NOT NULL,
    FOREIGN KEY(from_memory_id) REFERENCES memories(id),
    FOREIGN KEY(to_memory_id) REFERENCES memories(id),
    FOREIGN KEY(state_key_id) REFERENCES memory_state_keys(id),
    FOREIGN KEY(source_candidate_id) REFERENCES memory_candidates(id),
    FOREIGN KEY(source_operation_id) REFERENCES memory_operation_log(id)
);

CREATE INDEX IF NOT EXISTS idx_memory_edges_from
    ON memory_edges(from_memory_id, edge_type);

CREATE INDEX IF NOT EXISTS idx_memory_edges_to
    ON memory_edges(to_memory_id, edge_type);

CREATE INDEX IF NOT EXISTS idx_memory_edges_state
    ON memory_edges(state_key_id, edge_type, created_at_epoch);
