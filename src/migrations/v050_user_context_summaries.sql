-- v050_user_context_summaries: editable, source-backed profile summaries.

CREATE TABLE IF NOT EXISTS user_context_summaries (
    id INTEGER PRIMARY KEY,
    user_key TEXT NOT NULL DEFAULT 'user:default',
    owner_scope TEXT NOT NULL CHECK (
        owner_scope IN ('user', 'workspace', 'repo', 'session')
    ),
    owner_key TEXT NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('user', 'workspace', 'repo', 'project', 'session')),
    scope_key TEXT,
    summary_text TEXT NOT NULL,
    source_claim_ids_json TEXT NOT NULL CHECK (
        json_valid(source_claim_ids_json) = 1
        AND json_type(source_claim_ids_json) = 'array'
    ),
    source_memory_ids_json TEXT NOT NULL CHECK (
        json_valid(source_memory_ids_json) = 1
        AND json_type(source_memory_ids_json) = 'array'
    ),
    source_activity_refs_json TEXT NOT NULL CHECK (
        json_valid(source_activity_refs_json) = 1
        AND json_type(source_activity_refs_json) = 'array'
    ),
    status TEXT NOT NULL CHECK (status IN ('active', 'superseded', 'edited', 'deleted')),
    model TEXT,
    version INTEGER NOT NULL CHECK (version > 0),
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_user_context_summaries_owner_active
    ON user_context_summaries(owner_scope, owner_key, scope, scope_key, status, updated_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_user_context_summaries_user_recent
    ON user_context_summaries(user_key, updated_at_epoch DESC);
