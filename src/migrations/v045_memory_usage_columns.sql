ALTER TABLE memories ADD COLUMN last_accessed_epoch INTEGER;
ALTER TABLE memories ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_memories_usage
    ON memories(access_count DESC, last_accessed_epoch DESC);

CREATE TABLE IF NOT EXISTS memory_citation_events (
    id INTEGER PRIMARY KEY,
    host TEXT NOT NULL,
    project TEXT NOT NULL,
    session_id TEXT NOT NULL,
    source TEXT NOT NULL,
    message_hash TEXT NOT NULL,
    citation_line_present INTEGER NOT NULL DEFAULT 0,
    parsed_count INTEGER NOT NULL DEFAULT 0,
    matched_count INTEGER NOT NULL DEFAULT 0,
    inserted_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL CHECK (status IN ('no_citation', 'unmatched', 'matched')),
    created_at_epoch INTEGER NOT NULL,
    UNIQUE(host, project, session_id, source, message_hash)
);

CREATE TABLE IF NOT EXISTS memory_usage_events (
    id INTEGER PRIMARY KEY,
    citation_event_id INTEGER NOT NULL REFERENCES memory_citation_events(id) ON DELETE CASCADE,
    host TEXT NOT NULL,
    project TEXT NOT NULL,
    session_id TEXT NOT NULL,
    source TEXT NOT NULL,
    message_hash TEXT NOT NULL,
    memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    context_injection_item_id INTEGER REFERENCES context_injection_items(id) ON DELETE SET NULL,
    created_at_epoch INTEGER NOT NULL,
    UNIQUE(host, project, session_id, source, message_hash, memory_id)
);

CREATE INDEX IF NOT EXISTS idx_memory_citation_events_project_recent
    ON memory_citation_events(project, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_memory_usage_events_memory_recent
    ON memory_usage_events(memory_id, created_at_epoch DESC);
