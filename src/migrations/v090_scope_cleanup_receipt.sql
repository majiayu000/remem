-- v090_scope_cleanup_receipt: persist the exact scope-cleanup response so an
-- idempotent retry never reconstructs it from mutable rows or audit logs.

ALTER TABLE memory_operation_log
    ADD COLUMN activation_id TEXT CHECK (
        activation_id IS NULL OR (
            typeof(activation_id) = 'text'
            AND length(trim(activation_id)) > 0
            AND instr(activation_id, char(0)) = 0
        )
    );

ALTER TABLE memory_operation_log
    ADD COLUMN scope_cleanup_response_json TEXT;

CREATE UNIQUE INDEX idx_memory_operation_log_activation
    ON memory_operation_log(activation_id)
    WHERE activation_id IS NOT NULL;

CREATE TABLE memory_scope_cleanup_receipts (
    activation_id TEXT PRIMARY KEY
        REFERENCES memory_activation_requests(activation_id) ON DELETE RESTRICT,
    result_memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE RESTRICT,
    operation_id INTEGER NOT NULL REFERENCES memory_operation_log(id) ON DELETE RESTRICT,
    response_json TEXT NOT NULL CHECK (
        typeof(response_json) = 'text'
        AND json_valid(response_json)
        AND json_type(response_json) = 'object'
    ),
    created_at_epoch INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_memory_scope_cleanup_receipts_operation
    ON memory_scope_cleanup_receipts(operation_id);

CREATE TRIGGER memory_scope_cleanup_receipts_validate_insert
BEFORE INSERT ON memory_scope_cleanup_receipts
WHEN (
    EXISTS(
        SELECT 1 FROM memory_activation_requests activation
        JOIN memory_operation_log operation
          ON operation.id = NEW.operation_id
        WHERE activation.activation_id = NEW.activation_id
          AND activation.route_kind = 'scope_cleanup'
          AND activation.result_memory_id = NEW.result_memory_id
          AND operation.activation_id IS NULL
          AND operation.source = 'memory_cleanup'
          AND operation.result_memory_id = NEW.result_memory_id
          AND json(operation.superseded_ids) = json(activation.superseded_ids_json)
          AND operation.scope_cleanup_response_json = NEW.response_json
    )
    AND NOT EXISTS(
        SELECT 1 FROM memory_scope_cleanup_receipts existing
        WHERE existing.activation_id = NEW.activation_id
           OR existing.operation_id = NEW.operation_id
    )
    AND lower(NEW.response_json) NOT GLOB '*\ud[89ab][0-9a-f][0-9a-f]*'
    AND lower(NEW.response_json) NOT GLOB '*\ud[cdef][0-9a-f][0-9a-f]*'
    AND NOT EXISTS(
        WITH RECURSIVE response_characters(position, character) AS (
            SELECT 1, substr(NEW.response_json, 1, 1)
            UNION ALL
            SELECT position + 1, substr(NEW.response_json, position + 1, 1)
            FROM response_characters
            WHERE position < length(NEW.response_json)
        )
        SELECT 1 FROM response_characters
        WHERE length(CAST(character AS BLOB)) != CASE
            WHEN unicode(character) BETWEEN 0 AND 127 THEN 1
            WHEN unicode(character) BETWEEN 128 AND 2047 THEN 2
            WHEN unicode(character) BETWEEN 2048 AND 55295 THEN 3
            WHEN unicode(character) BETWEEN 57344 AND 65535 THEN 3
            WHEN unicode(character) BETWEEN 65536 AND 1114111 THEN 4
            ELSE -1
        END
    )
    AND (SELECT COUNT(*) FROM json_each(NEW.response_json)) = 5
    AND json_type(NEW.response_json, '$.current_id') = 'integer'
    AND json_extract(NEW.response_json, '$.current_id') = NEW.result_memory_id
    AND json_type(NEW.response_json, '$.operation_id') = 'integer'
    AND json_extract(NEW.response_json, '$.operation_id') = NEW.operation_id
    AND json_type(NEW.response_json, '$.stale_ids') = 'array'
    AND NOT EXISTS(
        SELECT 1 FROM json_each(NEW.response_json, '$.stale_ids') stale
        WHERE stale.type IS NOT 'integer' OR stale.value <= 0
    )
    AND json_type(NEW.response_json, '$.edge_count') = 'integer'
    AND json_extract(NEW.response_json, '$.edge_count') >= 0
    AND json_extract(NEW.response_json, '$.edge_count') <= 9223372036854775807
    AND json_type(NEW.response_json, '$.affected') = 'array'
    AND NOT EXISTS(
        SELECT 1 FROM json_each(NEW.response_json, '$.affected') affected
        WHERE affected.type IS NOT 'object'
           OR (SELECT COUNT(*) FROM json_each(affected.value)) != 6
           OR json_type(affected.value, '$.object_ref') IS NOT 'text'
           OR json_type(affected.value, '$.title') IS NOT 'text'
           OR json_type(affected.value, '$.previous_status') IS NOT 'text'
           OR json_type(affected.value, '$.new_status') IS NOT 'text'
           OR json_type(affected.value, '$.previous_owner') IS NOT 'object'
           OR json_type(affected.value, '$.new_owner') IS NOT 'object'
           OR (SELECT COUNT(*) FROM json_each(
                   json_extract(affected.value, '$.previous_owner')
              )) != 8
           OR (SELECT COUNT(*) FROM json_each(
                   json_extract(affected.value, '$.new_owner')
              )) != 8
           OR COALESCE(json_type(affected.value, '$.previous_owner.source_project'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.previous_owner.target_project'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.previous_owner.owner_scope'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.previous_owner.owner_key'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.previous_owner.topic_domain'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.previous_owner.routing_confidence'), 'missing')
                NOT IN ('integer', 'real', 'null')
           OR (
                json_type(affected.value, '$.previous_owner.routing_confidence')
                    IN ('integer', 'real')
                AND (
                    abs(json_extract(
                        affected.value,
                        '$.previous_owner.routing_confidence'
                    )) <= 1.7976931348623157e308
                ) IS NOT TRUE
           )
           OR COALESCE(json_type(affected.value, '$.previous_owner.routing_reason'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.previous_owner.context_class'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.new_owner.source_project'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.new_owner.target_project'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.new_owner.owner_scope'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.new_owner.owner_key'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.new_owner.topic_domain'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.new_owner.routing_confidence'), 'missing')
                NOT IN ('integer', 'real', 'null')
           OR (
                json_type(affected.value, '$.new_owner.routing_confidence')
                    IN ('integer', 'real')
                AND (
                    abs(json_extract(
                        affected.value,
                        '$.new_owner.routing_confidence'
                    )) <= 1.7976931348623157e308
                ) IS NOT TRUE
           )
           OR COALESCE(json_type(affected.value, '$.new_owner.routing_reason'), 'missing')
                NOT IN ('text', 'null')
           OR COALESCE(json_type(affected.value, '$.new_owner.context_class'), 'missing')
                NOT IN ('text', 'null')
    )
    AND json(json_extract(NEW.response_json, '$.stale_ids')) = json((
        SELECT superseded_ids_json FROM memory_activation_requests
        WHERE activation_id = NEW.activation_id
    ))
    AND json_extract(NEW.response_json, '$.edge_count') = (
        SELECT COUNT(*) FROM memory_edges
        WHERE edge_type = 'duplicates' AND source_operation_id = NEW.operation_id
    )
    AND json_array_length(json_extract(NEW.response_json, '$.affected')) =
        json_array_length(json_extract(NEW.response_json, '$.stale_ids')) + 1
) IS NOT TRUE
BEGIN
    SELECT RAISE(ABORT, 'invalid scope cleanup response receipt');
END;

CREATE TRIGGER memory_operation_log_activation_no_update
BEFORE UPDATE ON memory_operation_log
WHEN OLD.activation_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'activation-bound memory operation is immutable');
END;

CREATE TRIGGER memory_operation_log_activation_no_replace
BEFORE INSERT ON memory_operation_log
WHEN EXISTS(
    SELECT 1 FROM memory_operation_log existing
    WHERE existing.activation_id IS NOT NULL
      AND (
          existing.id = NEW.id
          OR existing.activation_id = NEW.activation_id
      )
)
BEGIN
    SELECT RAISE(ABORT, 'activation-bound memory operation is immutable');
END;

CREATE TRIGGER memory_operation_log_activation_validate_update
BEFORE UPDATE OF activation_id ON memory_operation_log
WHEN OLD.activation_id IS NULL AND NEW.activation_id IS NOT NULL AND (
    EXISTS(
        SELECT 1 FROM memory_activation_requests activation
        WHERE activation.activation_id = NEW.activation_id
          AND activation.route_kind = 'scope_cleanup'
          AND activation.result_memory_id = NEW.result_memory_id
          AND json(activation.superseded_ids_json) = json(NEW.superseded_ids)
    )
    AND NEW.scope_cleanup_response_json IS NOT NULL
    AND EXISTS(
        SELECT 1 FROM memory_scope_cleanup_receipts receipt
        WHERE receipt.activation_id = NEW.activation_id
          AND receipt.operation_id = NEW.id
          AND receipt.result_memory_id = NEW.result_memory_id
          AND receipt.response_json = NEW.scope_cleanup_response_json
    )
) IS NOT TRUE
BEGIN
    SELECT RAISE(ABORT, 'invalid cleanup operation activation binding');
END;

CREATE TRIGGER memory_operation_log_activation_no_delete
BEFORE DELETE ON memory_operation_log
WHEN OLD.activation_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'activation-bound memory operation is immutable');
END;

CREATE TRIGGER memory_edges_activation_bound_no_insert
BEFORE INSERT ON memory_edges
WHEN (
    NEW.source_operation_id IS NOT NULL AND EXISTS(
        SELECT 1 FROM memory_operation_log operation
        WHERE operation.id = NEW.source_operation_id
          AND operation.activation_id IS NOT NULL
    )
) OR EXISTS(
    SELECT 1 FROM memory_edges existing
    JOIN memory_operation_log operation
      ON operation.id = existing.source_operation_id
    WHERE existing.id = NEW.id
      AND operation.activation_id IS NOT NULL
)
BEGIN
    SELECT RAISE(ABORT, 'activation-bound memory edges are immutable');
END;

CREATE TRIGGER memory_edges_activation_bound_no_update
BEFORE UPDATE ON memory_edges
WHEN EXISTS(
    SELECT 1 FROM memory_operation_log operation
    WHERE operation.activation_id IS NOT NULL
      AND operation.id IN (OLD.source_operation_id, NEW.source_operation_id)
)
BEGIN
    SELECT RAISE(ABORT, 'activation-bound memory edges are immutable');
END;

CREATE TRIGGER memory_edges_activation_bound_no_delete
BEFORE DELETE ON memory_edges
WHEN OLD.source_operation_id IS NOT NULL AND EXISTS(
    SELECT 1 FROM memory_operation_log operation
    WHERE operation.id = OLD.source_operation_id
      AND operation.activation_id IS NOT NULL
)
BEGIN
    SELECT RAISE(ABORT, 'activation-bound memory edges are immutable');
END;

CREATE TRIGGER memory_scope_cleanup_receipts_no_update
BEFORE UPDATE ON memory_scope_cleanup_receipts
BEGIN
    SELECT RAISE(ABORT, 'scope cleanup response receipt is immutable');
END;

CREATE TRIGGER memory_scope_cleanup_receipts_no_delete
BEFORE DELETE ON memory_scope_cleanup_receipts
BEGIN
    SELECT RAISE(ABORT, 'scope cleanup response receipt is immutable');
END;
