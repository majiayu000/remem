-- v051_memory_suppressions_feedback: policy suppression and relevance feedback.

CREATE TABLE IF NOT EXISTS memory_suppressions (
    id INTEGER PRIMARY KEY,
    owner_scope TEXT CHECK (
        owner_scope IS NULL OR owner_scope IN ('user', 'workspace', 'repo', 'session')
    ),
    owner_key TEXT,
    target_kind TEXT NOT NULL CHECK (
        target_kind IN (
            'memory',
            'user_claim',
            'user_candidate',
            'topic_key',
            'entity',
            'pattern',
            'summary'
        )
    ),
    target_id INTEGER,
    target_value TEXT,
    reason TEXT NOT NULL,
    actor TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'revoked')),
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    CHECK (
        target_id IS NOT NULL
        OR (target_value IS NOT NULL AND length(trim(target_value)) > 0)
    ),
    CHECK (
        (target_kind IN ('memory', 'user_claim', 'user_candidate') AND target_id IS NOT NULL)
        OR (target_kind IN ('topic_key', 'entity', 'pattern') AND target_value IS NOT NULL)
        OR (target_kind = 'summary' AND (target_id IS NOT NULL OR target_value IS NOT NULL))
    )
);

CREATE INDEX IF NOT EXISTS idx_memory_suppressions_target_active
    ON memory_suppressions(target_kind, target_id, target_value, status);

CREATE INDEX IF NOT EXISTS idx_memory_suppressions_owner_active
    ON memory_suppressions(owner_scope, owner_key, status, updated_at_epoch DESC);

CREATE TABLE IF NOT EXISTS memory_feedback (
    id INTEGER PRIMARY KEY,
    target_kind TEXT NOT NULL CHECK (
        target_kind IN (
            'memory',
            'user_claim',
            'user_candidate',
            'topic_key',
            'entity',
            'pattern',
            'summary'
        )
    ),
    target_id INTEGER,
    target_value TEXT,
    feedback TEXT NOT NULL CHECK (
        feedback IN ('relevant', 'not_relevant', 'harmful', 'stale', 'too_noisy')
    ),
    source TEXT NOT NULL,
    context_injection_item_id INTEGER REFERENCES context_injection_items(id) ON DELETE SET NULL,
    session_id TEXT,
    project TEXT,
    reason TEXT,
    created_at_epoch INTEGER NOT NULL,
    CHECK (
        target_id IS NOT NULL
        OR (target_value IS NOT NULL AND length(trim(target_value)) > 0)
    ),
    CHECK (
        (target_kind IN ('memory', 'user_claim', 'user_candidate') AND target_id IS NOT NULL)
        OR (target_kind IN ('topic_key', 'entity', 'pattern') AND target_value IS NOT NULL)
        OR (target_kind = 'summary' AND (target_id IS NOT NULL OR target_value IS NOT NULL))
    )
);

CREATE INDEX IF NOT EXISTS idx_memory_feedback_target_recent
    ON memory_feedback(target_kind, target_id, target_value, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_memory_feedback_context_item
    ON memory_feedback(context_injection_item_id);
