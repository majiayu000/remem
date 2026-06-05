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
    source_event_ids TEXT NOT NULL DEFAULT '[]' CHECK (
        json_valid(source_event_ids) = 1
        AND json_type(source_event_ids) = 'array'
    ),
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
        (
            edge_type IN (
                'supersedes',
                'duplicates',
                'conflicts',
                'derived_from',
                'merged_into',
                'split_from',
                'similar_to',
                'candidate_hint',
                'co_occurs_with'
            )
            AND from_node_kind = to_node_kind
        )
        OR (
            edge_type = 'extracted_from'
            AND from_node_kind IN ('entity', 'fact', 'state', 'topic')
            AND to_node_kind = 'episode'
        )
        OR (
            edge_type = 'mentions'
            AND from_node_kind IN ('memory', 'episode')
            AND to_node_kind = 'entity'
        )
        OR (
            edge_type = 'has_state'
            AND from_node_kind = 'memory'
            AND to_node_kind = 'state'
        )
        OR (
            edge_type = 'has_topic'
            AND from_node_kind IN ('memory', 'episode')
            AND to_node_kind = 'topic'
        )
    ),
    CHECK (
        edge_trust != 'trusted'
        OR (
            source_event_ids IS NOT NULL
            AND json_array_length(source_event_ids) > 0
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

CREATE TRIGGER IF NOT EXISTS graph_edges_validate_source_events_insert
BEFORE INSERT ON graph_edges
WHEN NEW.edge_trust = 'trusted'
BEGIN
    SELECT CASE
        WHEN EXISTS (
            SELECT 1
            FROM json_each(NEW.source_event_ids) AS source_event
            WHERE source_event.type != 'integer'
               OR source_event.value <= 0
        )
        THEN RAISE(ABORT, 'graph_edges trusted source_event_ids must be positive integers')
    END;

    SELECT CASE
        WHEN EXISTS (
            SELECT 1
            FROM json_each(NEW.source_event_ids) AS source_event
            WHERE NOT EXISTS (
                SELECT 1 FROM captured_events WHERE id = source_event.value
            )
        )
        THEN RAISE(ABORT, 'graph_edges trusted source event missing')
    END;
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_validate_source_events_update
BEFORE UPDATE OF edge_trust, source_event_ids ON graph_edges
WHEN NEW.edge_trust = 'trusted'
BEGIN
    SELECT CASE
        WHEN EXISTS (
            SELECT 1
            FROM json_each(NEW.source_event_ids) AS source_event
            WHERE source_event.type != 'integer'
               OR source_event.value <= 0
        )
        THEN RAISE(ABORT, 'graph_edges trusted source_event_ids must be positive integers')
    END;

    SELECT CASE
        WHEN EXISTS (
            SELECT 1
            FROM json_each(NEW.source_event_ids) AS source_event
            WHERE NOT EXISTS (
                SELECT 1 FROM captured_events WHERE id = source_event.value
            )
        )
        THEN RAISE(ABORT, 'graph_edges trusted source event missing')
    END;
END;

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

CREATE TRIGGER IF NOT EXISTS graph_edges_memories_delete
AFTER DELETE ON memories
BEGIN
    DELETE FROM graph_edges
    WHERE (from_node_kind = 'memory' AND from_node_id = OLD.id)
       OR (to_node_kind = 'memory' AND to_node_id = OLD.id);
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_entities_delete
AFTER DELETE ON entities
BEGIN
    DELETE FROM graph_edges
    WHERE (from_node_kind = 'entity' AND from_node_id = OLD.id)
       OR (to_node_kind = 'entity' AND to_node_id = OLD.id);
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_memory_facts_delete
AFTER DELETE ON memory_facts
BEGIN
    DELETE FROM graph_edges
    WHERE (from_node_kind = 'fact' AND from_node_id = OLD.id)
       OR (to_node_kind = 'fact' AND to_node_id = OLD.id);
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_captured_events_delete
AFTER DELETE ON captured_events
BEGIN
    DELETE FROM graph_edges
    WHERE (from_node_kind = 'episode' AND from_node_id = OLD.id)
       OR (to_node_kind = 'episode' AND to_node_id = OLD.id)
       OR EXISTS (
           SELECT 1
           FROM json_each(graph_edges.source_event_ids) AS source_event
           WHERE source_event.value = OLD.id
       );
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_topic_segments_delete
AFTER DELETE ON topic_segments
BEGIN
    DELETE FROM graph_edges
    WHERE (from_node_kind = 'topic' AND from_node_id = OLD.id)
       OR (to_node_kind = 'topic' AND to_node_id = OLD.id);
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
