-- v052_user_context_candidates: review-gated user-context extraction candidates.

CREATE TABLE IF NOT EXISTS user_context_candidates (
    id INTEGER PRIMARY KEY,
    user_key TEXT NOT NULL DEFAULT 'user:default',
    owner_scope TEXT NOT NULL CHECK (
        owner_scope IN ('user', 'workspace', 'repo', 'session')
    ),
    owner_key TEXT NOT NULL,
    source_project TEXT,
    host TEXT,
    session_id TEXT,
    claim_type TEXT NOT NULL CHECK (
        claim_type IN (
            'identity',
            'role',
            'preference',
            'skill',
            'goal',
            'project',
            'relationship',
            'constraint',
            'activity'
        )
    ),
    claim_key TEXT,
    claim_text TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    sensitivity TEXT NOT NULL CHECK (
        sensitivity IN ('normal', 'personal', 'sensitive', 'restricted')
    ),
    risk_class TEXT NOT NULL CHECK (risk_class IN ('low', 'medium', 'high')),
    source_kind TEXT NOT NULL,
    source_refs_json TEXT NOT NULL CHECK (
        json_valid(source_refs_json) = 1
        AND json_type(source_refs_json) = 'array'
        AND json_array_length(source_refs_json) > 0
    ),
    source_preview TEXT,
    review_status TEXT NOT NULL CHECK (
        review_status IN (
            'pending_review',
            'auto_promoted',
            'approved',
            'edited',
            'rejected',
            'suppressed',
            'deferred'
        )
    ),
    auto_promote_block_reason TEXT,
    review_note TEXT,
    result_claim_id INTEGER,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    FOREIGN KEY(result_claim_id) REFERENCES user_context_claims(id)
);

CREATE INDEX IF NOT EXISTS idx_user_context_candidates_inbox
    ON user_context_candidates(review_status, updated_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_user_context_candidates_user_recent
    ON user_context_candidates(user_key, updated_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_user_context_candidates_dedupe
    ON user_context_candidates(owner_scope, owner_key, claim_type, claim_key, review_status);
