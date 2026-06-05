-- v030_dream_cluster_decisions: durable Dream consolidation decisions.

CREATE TABLE IF NOT EXISTS dream_cluster_decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    cluster_signature TEXT NOT NULL,
    decision TEXT NOT NULL CHECK(decision IN ('merged', 'no_merge', 'defer', 'failed')),
    reason TEXT,
    member_ids_json TEXT NOT NULL,
    cluster_size INTEGER NOT NULL,
    next_review_epoch INTEGER,
    source_memory_id INTEGER,
    source_operation_id INTEGER,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    last_seen_epoch INTEGER NOT NULL,
    UNIQUE(project, memory_type, cluster_signature),
    FOREIGN KEY(source_memory_id) REFERENCES memories(id),
    FOREIGN KEY(source_operation_id) REFERENCES memory_operation_log(id)
);

CREATE INDEX IF NOT EXISTS idx_dream_cluster_decisions_review
    ON dream_cluster_decisions(project, decision, next_review_epoch);

CREATE INDEX IF NOT EXISTS idx_dream_cluster_decisions_signature
    ON dream_cluster_decisions(project, memory_type, cluster_signature);
