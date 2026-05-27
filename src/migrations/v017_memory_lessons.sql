-- v017_memory_lessons: first-class metadata for lesson memories.
--
-- The memory body remains in `memories` with memory_type='lesson' so search,
-- dedup, branch, scope, and local tooling keep working. This side table stores
-- lesson-specific lifecycle signals used for reinforcement and bounded context
-- injection.

CREATE TABLE IF NOT EXISTS memory_lessons (
    memory_id INTEGER PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
    confidence REAL NOT NULL DEFAULT 0.7 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    reinforcement_count INTEGER NOT NULL DEFAULT 1 CHECK (reinforcement_count >= 1),
    source_evidence TEXT,
    last_reinforced_at_epoch INTEGER NOT NULL,
    stale_after_epoch INTEGER
);

CREATE INDEX IF NOT EXISTS idx_memory_lessons_rank
ON memory_lessons(confidence DESC, reinforcement_count DESC, last_reinforced_at_epoch DESC);
