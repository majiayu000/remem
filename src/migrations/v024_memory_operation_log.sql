-- v024_memory_operation_log: lifecycle operation planner audit trail.
--
-- This table records why a durable memory write became add/update/noop/defer.
-- It is an explanation layer, not the source of truth for memory contents.

CREATE TABLE IF NOT EXISTS memory_operation_log (
    id INTEGER PRIMARY KEY,
    operation TEXT NOT NULL,
    planner_version TEXT NOT NULL,
    actor TEXT NOT NULL,
    source TEXT NOT NULL,
    owner_scope TEXT,
    owner_key TEXT,
    memory_type TEXT,
    state_key TEXT,
    input_topic_key TEXT,
    source_candidate_id INTEGER,
    result_memory_id INTEGER,
    superseded_ids TEXT NOT NULL DEFAULT '[]',
    conflicting_ids TEXT NOT NULL DEFAULT '[]',
    noop_reason TEXT,
    defer_reason TEXT,
    confidence REAL,
    reason TEXT,
    created_at_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_operation_log_state
    ON memory_operation_log(owner_scope, owner_key, memory_type, state_key, created_at_epoch);
