-- v049_user_context_claims: explicit, auditable user-context claims.

CREATE TABLE IF NOT EXISTS user_context_claims (
    id INTEGER PRIMARY KEY,
    user_key TEXT NOT NULL DEFAULT 'user:default',
    owner_scope TEXT NOT NULL CHECK (
        owner_scope IN ('user', 'workspace', 'repo', 'session')
    ),
    owner_key TEXT NOT NULL,
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
    claim_key TEXT NOT NULL,
    claim_text TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    sensitivity TEXT NOT NULL CHECK (
        sensitivity IN ('normal', 'personal', 'sensitive', 'restricted')
    ),
    source_kind TEXT NOT NULL,
    source_refs_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN (
            'active',
            'pending_review',
            'stale',
            'superseded',
            'suppressed',
            'rejected',
            'deleted'
        )
    ),
    valid_from_epoch INTEGER,
    valid_to_epoch INTEGER,
    last_confirmed_at_epoch INTEGER,
    supersedes_claim_id INTEGER,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    FOREIGN KEY(supersedes_claim_id) REFERENCES user_context_claims(id)
);

CREATE INDEX IF NOT EXISTS idx_user_context_claims_owner_active
    ON user_context_claims(owner_scope, owner_key, claim_type, claim_key, status);

CREATE INDEX IF NOT EXISTS idx_user_context_claims_user_recent
    ON user_context_claims(user_key, updated_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_user_context_claims_status
    ON user_context_claims(status, updated_at_epoch DESC);
