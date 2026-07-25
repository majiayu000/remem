-- v060_memory_poisoning_defense: durable source trust and quarantine metadata.

ALTER TABLE memory_candidates ADD COLUMN source_trust_class TEXT NOT NULL DEFAULT 'local_tool_output';
ALTER TABLE memory_candidates ADD COLUMN quarantine_pattern_id TEXT;
ALTER TABLE memory_candidates ADD COLUMN quarantine_pattern_version INTEGER;
ALTER TABLE memory_candidates ADD COLUMN acknowledged_pattern_id TEXT;
ALTER TABLE memory_candidates ADD COLUMN acknowledged_pattern_version INTEGER;
ALTER TABLE memory_candidates ADD COLUMN acknowledged_at_epoch INTEGER;

ALTER TABLE memories ADD COLUMN source_trust_class TEXT NOT NULL DEFAULT 'local_tool_output';
ALTER TABLE memories ADD COLUMN acknowledged_pattern_id TEXT;
ALTER TABLE memories ADD COLUMN acknowledged_pattern_version INTEGER;
ALTER TABLE memories ADD COLUMN acknowledged_at_epoch INTEGER;

CREATE INDEX IF NOT EXISTS idx_memory_candidates_quarantine
    ON memory_candidates(review_status, quarantine_pattern_id, created_at_epoch)
    WHERE quarantine_pattern_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_memories_source_trust
    ON memories(source_trust_class, status, updated_at_epoch DESC);
