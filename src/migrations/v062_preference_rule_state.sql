-- v062_preference_rule_state: canonical state for preference-derived rules.
--
-- This migration only creates durable state. It does not enable compilation,
-- write rule artifacts, or add hook-side enforcement.

CREATE TABLE IF NOT EXISTS memory_preference_reinforcements (
    memory_id INTEGER PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
    reinforcement_count INTEGER NOT NULL DEFAULT 1 CHECK (reinforcement_count >= 1),
    source_evidence TEXT,
    last_reinforced_at_epoch INTEGER NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_preference_reinforcements_rank
    ON memory_preference_reinforcements(
        reinforcement_count DESC,
        last_reinforced_at_epoch DESC
    );

CREATE TABLE IF NOT EXISTS preference_rule_overrides (
    id INTEGER PRIMARY KEY,
    project TEXT NOT NULL,
    rule_id TEXT NOT NULL,
    source_memory_id INTEGER REFERENCES memories(id) ON DELETE SET NULL,
    disabled INTEGER NOT NULL DEFAULT 0 CHECK (disabled IN (0, 1)),
    action_override TEXT CHECK (
        action_override IS NULL OR action_override IN ('warn', 'block')
    ),
    reason TEXT,
    updated_by TEXT NOT NULL DEFAULT 'user',
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(project, rule_id)
);

CREATE INDEX IF NOT EXISTS idx_preference_rule_overrides_project
    ON preference_rule_overrides(project, disabled, updated_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_preference_rule_overrides_source
    ON preference_rule_overrides(source_memory_id)
    WHERE source_memory_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS preference_rule_diagnostics (
    id INTEGER PRIMARY KEY,
    project TEXT NOT NULL,
    event_kind TEXT NOT NULL CHECK (event_kind IN ('compile', 'evaluation')),
    status TEXT NOT NULL CHECK (status IN ('ok', 'warn', 'error')),
    message TEXT,
    rule_id TEXT,
    artifact_path TEXT,
    rule_count INTEGER CHECK (rule_count IS NULL OR rule_count >= 0),
    occurred_at_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_preference_rule_diagnostics_project_event
    ON preference_rule_diagnostics(project, event_kind, occurred_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_preference_rule_diagnostics_rule
    ON preference_rule_diagnostics(rule_id, occurred_at_epoch DESC)
    WHERE rule_id IS NOT NULL;
