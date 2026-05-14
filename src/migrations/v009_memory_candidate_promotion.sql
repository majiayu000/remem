-- v009_memory_candidate_promotion: preserve candidate evidence on promoted memories.

ALTER TABLE memories ADD COLUMN evidence_event_ids TEXT;
ALTER TABLE memories ADD COLUMN source_candidate_id INTEGER;
ALTER TABLE memories ADD COLUMN confidence REAL;

CREATE INDEX IF NOT EXISTS idx_memories_source_candidate
    ON memories(source_candidate_id);

CREATE INDEX IF NOT EXISTS idx_memory_candidates_dedupe
    ON memory_candidates(project_id, scope, memory_type, topic_key, text);
