-- v033_graph_candidates: governed graph candidate queue.
--
-- LLM output lands in graph_candidates first. Promotion writes to the typed
-- graph_edges contract from v031, so model output cannot mutate the trusted
-- graph directly.

CREATE TABLE IF NOT EXISTS graph_candidates (
    id INTEGER PRIMARY KEY,
    project_id INTEGER REFERENCES projects(id),
    source_project TEXT NOT NULL,
    candidate_type TEXT NOT NULL CHECK(candidate_type = 'edge'),
    edge_type TEXT NOT NULL,
    from_ref TEXT NOT NULL,
    to_ref TEXT NOT NULL,
    evidence_event_ids TEXT NOT NULL,
    confidence REAL NOT NULL CHECK(confidence >= 0.0 AND confidence <= 1.0),
    risk_class TEXT NOT NULL CHECK(risk_class IN ('low', 'medium', 'high')),
    reason TEXT NOT NULL,
    review_status TEXT NOT NULL CHECK(review_status IN (
        'pending_review',
        'auto_promoted',
        'approved',
        'rejected',
        'deferred',
        'failed'
    )),
    review_note TEXT,
    promoted_edge_id INTEGER,
    source_operation_id INTEGER,
    failure_reason TEXT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    FOREIGN KEY(source_operation_id) REFERENCES memory_operation_log(id)
);

CREATE INDEX IF NOT EXISTS idx_graph_candidates_review
    ON graph_candidates(review_status, created_at_epoch, id);

CREATE INDEX IF NOT EXISTS idx_graph_candidates_project
    ON graph_candidates(project_id, candidate_type, edge_type);

CREATE UNIQUE INDEX IF NOT EXISTS idx_graph_candidates_dedupe
    ON graph_candidates(project_id, candidate_type, edge_type, from_ref, to_ref, evidence_event_ids);
