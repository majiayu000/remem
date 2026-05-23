-- v013_memory_temporal_facts: SQLite-native temporal facts with provenance.
--
-- This is intentionally a small relational layer, not a graph database. Facts
-- carry subject/predicate/object, validity time, learned time, provenance links,
-- confidence, and soft supersession.

CREATE TABLE IF NOT EXISTS memory_facts (
    id INTEGER PRIMARY KEY,
    project TEXT NOT NULL,
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL CHECK (
        predicate IN (
            'fixed_by',
            'verified_by',
            'supersedes',
            'blocked_by',
            'uses_file',
            'uses_command',
            'affects_project'
        )
    ),
    object TEXT NOT NULL,
    valid_from_epoch INTEGER,
    valid_to_epoch INTEGER,
    learned_at_epoch INTEGER NOT NULL,
    source_memory_id INTEGER REFERENCES memories(id) ON DELETE SET NULL,
    source_observation_id INTEGER REFERENCES observations(id) ON DELETE SET NULL,
    source_event_ids TEXT NOT NULL DEFAULT '[]',
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    supersedes_fact_id INTEGER REFERENCES memory_facts(id) ON DELETE SET NULL,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'stale')),
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    CHECK (
        valid_to_epoch IS NULL
        OR valid_from_epoch IS NULL
        OR valid_to_epoch >= valid_from_epoch
    )
);

CREATE INDEX IF NOT EXISTS idx_memory_facts_current
    ON memory_facts(project, subject, predicate, status, valid_to_epoch);

CREATE INDEX IF NOT EXISTS idx_memory_facts_as_of
    ON memory_facts(project, subject, predicate, valid_from_epoch, valid_to_epoch);

CREATE INDEX IF NOT EXISTS idx_memory_facts_source_memory
    ON memory_facts(source_memory_id);

CREATE INDEX IF NOT EXISTS idx_memory_facts_source_observation
    ON memory_facts(source_observation_id);

CREATE INDEX IF NOT EXISTS idx_memory_facts_supersedes
    ON memory_facts(supersedes_fact_id);
