-- v022_memory_state_keys: stable identity slots for mutable memories.
--
-- This migration is metadata-only. It creates current-state identity tables and
-- nullable links, but it does not merge, stale, or otherwise mutate existing
-- historical memory rows.

CREATE TABLE memory_state_keys (
    id INTEGER PRIMARY KEY,
    owner_scope TEXT NOT NULL,
    owner_key TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    state_key TEXT NOT NULL,
    state_label TEXT,
    state_status TEXT NOT NULL DEFAULT 'active',
    current_memory_id INTEGER,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(owner_scope, owner_key, memory_type, state_key),
    FOREIGN KEY(current_memory_id) REFERENCES memories(id)
);

CREATE INDEX idx_memory_state_keys_owner
    ON memory_state_keys(owner_scope, owner_key, memory_type, state_status);

CREATE INDEX idx_memory_state_keys_current
    ON memory_state_keys(current_memory_id)
    WHERE current_memory_id IS NOT NULL;

ALTER TABLE memories ADD COLUMN state_key_id INTEGER;

ALTER TABLE memory_candidates ADD COLUMN state_key TEXT;
ALTER TABLE memory_candidates ADD COLUMN state_key_confidence REAL;
ALTER TABLE memory_candidates ADD COLUMN state_key_reason TEXT;

CREATE INDEX idx_memories_state_key_id
    ON memories(state_key_id)
    WHERE state_key_id IS NOT NULL;

CREATE INDEX idx_memory_candidates_state_key
    ON memory_candidates(owner_scope, owner_key, memory_type, state_key)
    WHERE state_key IS NOT NULL;
