-- v037_graph_edge_source_candidate_integrity: restore storage-level
-- provenance integrity for graph_edges.source_candidate_id after the v034 table
-- rebuild dropped the original v031 invariant. The candidate id is typed by the
-- required memory_operation_log.source: memory_candidate ids live in
-- memory_candidates, and graph_candidate ids live in graph_candidates.

DROP TABLE IF EXISTS temp.graph_edge_source_candidate_integrity_check;
CREATE TEMP TABLE graph_edge_source_candidate_integrity_check (
    ok INTEGER NOT NULL CHECK(ok = 1)
);

INSERT INTO graph_edge_source_candidate_integrity_check(ok)
SELECT 0
FROM graph_edges e
WHERE e.source_candidate_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM memory_operation_log op
      WHERE op.id = e.source_operation_id
        AND op.source_candidate_id = e.source_candidate_id
        AND (
            (
                op.source = 'memory_candidate'
                AND EXISTS (
                    SELECT 1
                    FROM memory_candidates c
                    WHERE c.id = e.source_candidate_id
                )
            )
            OR (
                op.source = 'graph_candidate'
                AND EXISTS (
                    SELECT 1
                    FROM graph_candidates c
                    WHERE c.id = e.source_candidate_id
                )
            )
        )
  )
LIMIT 1;

DROP TABLE temp.graph_edge_source_candidate_integrity_check;

CREATE TRIGGER IF NOT EXISTS graph_edges_validate_source_candidate_insert
BEFORE INSERT ON graph_edges
WHEN NEW.source_candidate_id IS NOT NULL
BEGIN
    SELECT CASE
        WHEN NOT EXISTS (
            SELECT 1
            FROM memory_operation_log op
            WHERE op.id = NEW.source_operation_id
              AND op.source_candidate_id = NEW.source_candidate_id
              AND (
                  (
                      op.source = 'memory_candidate'
                      AND EXISTS (
                          SELECT 1
                          FROM memory_candidates c
                          WHERE c.id = NEW.source_candidate_id
                      )
                  )
                  OR (
                      op.source = 'graph_candidate'
                      AND EXISTS (
                          SELECT 1
                          FROM graph_candidates c
                          WHERE c.id = NEW.source_candidate_id
                      )
                  )
              )
        )
        THEN RAISE(ABORT, 'graph_edges source candidate missing')
    END;
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_validate_source_candidate_update
BEFORE UPDATE OF source_candidate_id, source_operation_id ON graph_edges
WHEN NEW.source_candidate_id IS NOT NULL
BEGIN
    SELECT CASE
        WHEN NOT EXISTS (
            SELECT 1
            FROM memory_operation_log op
            WHERE op.id = NEW.source_operation_id
              AND op.source_candidate_id = NEW.source_candidate_id
              AND (
                  (
                      op.source = 'memory_candidate'
                      AND EXISTS (
                          SELECT 1
                          FROM memory_candidates c
                          WHERE c.id = NEW.source_candidate_id
                      )
                  )
                  OR (
                      op.source = 'graph_candidate'
                      AND EXISTS (
                          SELECT 1
                          FROM graph_candidates c
                          WHERE c.id = NEW.source_candidate_id
                      )
                  )
              )
        )
        THEN RAISE(ABORT, 'graph_edges source candidate missing')
    END;
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_memory_candidates_delete
BEFORE DELETE ON memory_candidates
WHEN EXISTS (
    SELECT 1
    FROM graph_edges e
    JOIN memory_operation_log op ON op.id = e.source_operation_id
    WHERE e.source_candidate_id = OLD.id
      AND op.source = 'memory_candidate'
)
BEGIN
    SELECT RAISE(ABORT, 'graph_edges source candidate in use');
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_memory_candidates_update_id
BEFORE UPDATE OF id ON memory_candidates
WHEN NEW.id != OLD.id
  AND EXISTS (
      SELECT 1
      FROM graph_edges e
      JOIN memory_operation_log op ON op.id = e.source_operation_id
      WHERE e.source_candidate_id = OLD.id
        AND op.source = 'memory_candidate'
  )
BEGIN
    SELECT RAISE(ABORT, 'graph_edges source candidate in use');
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_graph_candidates_delete
BEFORE DELETE ON graph_candidates
WHEN EXISTS (
    SELECT 1
    FROM graph_edges e
    JOIN memory_operation_log op ON op.id = e.source_operation_id
    WHERE e.source_candidate_id = OLD.id
      AND op.source = 'graph_candidate'
)
BEGIN
    SELECT RAISE(ABORT, 'graph_edges source candidate in use');
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_graph_candidates_update_id
BEFORE UPDATE OF id ON graph_candidates
WHEN NEW.id != OLD.id
  AND EXISTS (
      SELECT 1
      FROM graph_edges e
      JOIN memory_operation_log op ON op.id = e.source_operation_id
      WHERE e.source_candidate_id = OLD.id
        AND op.source = 'graph_candidate'
  )
BEGIN
    SELECT RAISE(ABORT, 'graph_edges source candidate in use');
END;

CREATE TRIGGER IF NOT EXISTS graph_edges_memory_operation_provenance_update
BEFORE UPDATE OF source, source_candidate_id ON memory_operation_log
WHEN (NEW.source != OLD.source OR NEW.source_candidate_id IS NOT OLD.source_candidate_id)
  AND EXISTS (
      SELECT 1
      FROM graph_edges e
      WHERE e.source_operation_id = OLD.id
        AND e.source_candidate_id IS NOT NULL
  )
BEGIN
    SELECT RAISE(ABORT, 'graph_edges source operation in use');
END;
