-- v031_graph_candidates: governed graph candidate queue and trusted graph edges.
--
-- LLM output lands in graph_candidates first. Trusted graph_edges require either
-- a graph candidate or an explicit operation log, so model output cannot mutate
-- the trusted graph directly.

CREATE TABLE IF NOT EXISTS graph_candidates (
    id INTEGER PRIMARY KEY,
    project_id INTEGER REFERENCES projects(id),
    source_project TEXT NOT NULL,
    candidate_type TEXT NOT NULL CHECK(candidate_type IN (
        'entity_alias',
        'claim',
        'edge',
        'state_relation'
    )),
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

CREATE TABLE IF NOT EXISTS graph_edges (
    id INTEGER PRIMARY KEY,
    edge_type TEXT NOT NULL,
    from_ref TEXT NOT NULL,
    to_ref TEXT NOT NULL,
    source_candidate_id INTEGER,
    evidence_event_ids TEXT NOT NULL,
    source_operation_id INTEGER,
    confidence REAL,
    reason TEXT,
    created_at_epoch INTEGER NOT NULL,
    CHECK(source_candidate_id IS NOT NULL OR source_operation_id IS NOT NULL),
    FOREIGN KEY(source_candidate_id) REFERENCES graph_candidates(id),
    FOREIGN KEY(source_operation_id) REFERENCES memory_operation_log(id)
);

CREATE INDEX IF NOT EXISTS idx_graph_candidates_review
    ON graph_candidates(review_status, created_at_epoch, id);

CREATE INDEX IF NOT EXISTS idx_graph_candidates_project
    ON graph_candidates(project_id, candidate_type, edge_type);

CREATE UNIQUE INDEX IF NOT EXISTS idx_graph_candidates_dedupe
    ON graph_candidates(project_id, candidate_type, edge_type, from_ref, to_ref, evidence_event_ids);

CREATE INDEX IF NOT EXISTS idx_graph_edges_refs
    ON graph_edges(from_ref, edge_type, to_ref);

CREATE INDEX IF NOT EXISTS idx_graph_edges_candidate
    ON graph_edges(source_candidate_id)
    WHERE source_candidate_id IS NOT NULL;
