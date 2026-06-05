-- v031_graph_edges: first-class typed graph contract.
--
-- memory_edges remains the memory-to-memory lifecycle relation table. graph_edges
-- is the typed cross-node contract for future traversal; this migration adds the
-- storage contract only and does not change retrieval behavior.

CREATE TABLE IF NOT EXISTS graph_edges (
    id INTEGER PRIMARY KEY,
    edge_type TEXT NOT NULL CHECK (
        edge_type IN (
            'supersedes',
            'duplicates',
            'conflicts',
            'derived_from',
            'merged_into',
            'split_from',
            'extracted_from',
            'mentions',
            'has_state',
            'has_topic',
            'similar_to',
            'candidate_hint',
            'co_occurs_with'
        )
    ),
    edge_trust TEXT NOT NULL CHECK (edge_trust IN ('trusted', 'diagnostic_hint')),
    from_node_kind TEXT NOT NULL CHECK (
        from_node_kind IN ('memory', 'entity', 'fact', 'episode', 'state', 'topic')
    ),
    from_node_id INTEGER NOT NULL CHECK (from_node_id > 0),
    to_node_kind TEXT NOT NULL CHECK (
        to_node_kind IN ('memory', 'entity', 'fact', 'episode', 'state', 'topic')
    ),
    to_node_id INTEGER NOT NULL CHECK (to_node_id > 0),
    source_event_ids TEXT NOT NULL DEFAULT '[]',
    source_candidate_id INTEGER,
    source_operation_id INTEGER,
    confidence REAL CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
    reason TEXT,
    valid_from_epoch INTEGER,
    valid_to_epoch INTEGER,
    created_at_epoch INTEGER NOT NULL,
    CHECK (
        valid_to_epoch IS NULL
        OR valid_from_epoch IS NULL
        OR valid_to_epoch >= valid_from_epoch
    ),
    CHECK (
        (
            edge_type IN (
                'supersedes',
                'duplicates',
                'conflicts',
                'derived_from',
                'merged_into',
                'split_from',
                'extracted_from',
                'mentions',
                'has_state',
                'has_topic'
            )
            AND edge_trust = 'trusted'
        )
        OR (
            edge_type IN ('similar_to', 'candidate_hint', 'co_occurs_with')
            AND edge_trust = 'diagnostic_hint'
        )
    ),
    CHECK (
        edge_trust != 'trusted'
        OR (
            source_event_ids IS NOT NULL
            AND length(trim(source_event_ids)) > 2
            AND source_event_ids != '[]'
            AND source_candidate_id IS NOT NULL
            AND source_operation_id IS NOT NULL
            AND confidence IS NOT NULL
            AND reason IS NOT NULL
            AND length(trim(reason)) > 0
        )
    ),
    FOREIGN KEY(source_candidate_id) REFERENCES memory_candidates(id),
    FOREIGN KEY(source_operation_id) REFERENCES memory_operation_log(id)
);

CREATE INDEX IF NOT EXISTS idx_graph_edges_from
    ON graph_edges(from_node_kind, from_node_id, edge_type, created_at_epoch);

CREATE INDEX IF NOT EXISTS idx_graph_edges_to
    ON graph_edges(to_node_kind, to_node_id, edge_type, created_at_epoch);

CREATE INDEX IF NOT EXISTS idx_graph_edges_type
    ON graph_edges(edge_type, edge_trust, created_at_epoch);

CREATE TRIGGER IF NOT EXISTS graph_edges_validate_nodes_insert
BEFORE INSERT ON graph_edges
BEGIN
    SELECT CASE
        WHEN NEW.from_node_kind = 'memory'
             AND NOT EXISTS (SELECT 1 FROM memories WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from memory node missing')
        WHEN NEW.from_node_kind = 'entity'
             AND NOT EXISTS (SELECT 1 FROM entities WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from entity node missing')
        WHEN NEW.from_node_kind = 'fact'
             AND NOT EXISTS (SELECT 1 FROM memory_facts WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from fact node missing')
        WHEN NEW.from_node_kind = 'episode'
             AND NOT EXISTS (SELECT 1 FROM captured_events WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from episode node missing')
        WHEN NEW.from_node_kind = 'state'
             AND NOT EXISTS (SELECT 1 FROM memory_state_keys WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from state node missing')
        WHEN NEW.from_node_kind = 'topic'
             AND NOT EXISTS (SELECT 1 FROM topic_segments WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from topic node missing')
    END;

    SELECT CASE
        WHEN NEW.to_node_kind = 'memory'
             AND NOT EXISTS (SELECT 1 FROM memories WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to memory node missing')
        WHEN NEW.to_node_kind = 'entity'
             AND NOT EXISTS (SELECT 1 FROM entities WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to entity node missing')
        WHEN NEW.to_node_kind = 'fact'
             AND NOT EXISTS (SELECT 1 FROM memory_facts WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to fact node missing')
        WHEN NEW.to_node_kind = 'episode'
             AND NOT EXISTS (SELECT 1 FROM captured_events WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to episode node missing')
        WHEN NEW.to_node_kind = 'state'
             AND NOT EXISTS (SELECT 1 FROM memory_state_keys WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to state node missing')
        WHEN NEW.to_node_kind = 'topic'
             AND NOT EXISTS (SELECT 1 FROM topic_segments WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to topic node missing')
    END;
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_validate_nodes_update
BEFORE UPDATE OF from_node_kind, from_node_id, to_node_kind, to_node_id ON graph_edges
BEGIN
    SELECT CASE
        WHEN NEW.from_node_kind = 'memory'
             AND NOT EXISTS (SELECT 1 FROM memories WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from memory node missing')
        WHEN NEW.from_node_kind = 'entity'
             AND NOT EXISTS (SELECT 1 FROM entities WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from entity node missing')
        WHEN NEW.from_node_kind = 'fact'
             AND NOT EXISTS (SELECT 1 FROM memory_facts WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from fact node missing')
        WHEN NEW.from_node_kind = 'episode'
             AND NOT EXISTS (SELECT 1 FROM captured_events WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from episode node missing')
        WHEN NEW.from_node_kind = 'state'
             AND NOT EXISTS (SELECT 1 FROM memory_state_keys WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from state node missing')
        WHEN NEW.from_node_kind = 'topic'
             AND NOT EXISTS (SELECT 1 FROM topic_segments WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from topic node missing')
    END;

    SELECT CASE
        WHEN NEW.to_node_kind = 'memory'
             AND NOT EXISTS (SELECT 1 FROM memories WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to memory node missing')
        WHEN NEW.to_node_kind = 'entity'
             AND NOT EXISTS (SELECT 1 FROM entities WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to entity node missing')
        WHEN NEW.to_node_kind = 'fact'
             AND NOT EXISTS (SELECT 1 FROM memory_facts WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to fact node missing')
        WHEN NEW.to_node_kind = 'episode'
             AND NOT EXISTS (SELECT 1 FROM captured_events WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to episode node missing')
        WHEN NEW.to_node_kind = 'state'
             AND NOT EXISTS (SELECT 1 FROM memory_state_keys WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to state node missing')
        WHEN NEW.to_node_kind = 'topic'
             AND NOT EXISTS (SELECT 1 FROM topic_segments WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to topic node missing')
    END;
END;
