CREATE TABLE IF NOT EXISTS memory_poisoning_injection_drops (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_id INTEGER NOT NULL,
    pattern_id TEXT NOT NULL,
    pattern_version INTEGER NOT NULL,
    source_trust_class TEXT NOT NULL DEFAULT 'local_tool_output',
    source_project TEXT,
    title TEXT,
    created_at_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_poisoning_drops_created
    ON memory_poisoning_injection_drops(created_at_epoch DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_memory_poisoning_drops_pattern
    ON memory_poisoning_injection_drops(pattern_id, pattern_version, created_at_epoch DESC);
