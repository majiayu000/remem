-- v026_memory_claims: short-lived save_memory claims and summary candidate
-- noop audit rows for claim-covered duplicates.

CREATE TABLE IF NOT EXISTS memory_claims (
    id INTEGER PRIMARY KEY,
    memory_id INTEGER NOT NULL,
    project TEXT NOT NULL,
    source_project TEXT,
    session_id TEXT,
    host TEXT,
    claim_source TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    topic_key TEXT,
    title TEXT,
    content_fingerprint TEXT NOT NULL,
    content_preview TEXT NOT NULL,
    owner_scope TEXT,
    owner_key TEXT,
    branch TEXT,
    created_at_epoch INTEGER NOT NULL,
    expires_at_epoch INTEGER,
    consumed_at_epoch INTEGER,
    consumed_by_session_id TEXT,
    consumed_reason TEXT,
    FOREIGN KEY(memory_id) REFERENCES memories(id)
);

CREATE INDEX IF NOT EXISTS idx_memory_claims_session
    ON memory_claims(project, session_id, created_at_epoch);

CREATE INDEX IF NOT EXISTS idx_memory_claims_recent
    ON memory_claims(project, created_at_epoch);

CREATE INDEX IF NOT EXISTS idx_memory_claims_fingerprint
    ON memory_claims(project, memory_type, content_fingerprint);

CREATE TABLE IF NOT EXISTS memory_candidate_noops (
    id INTEGER PRIMARY KEY,
    project TEXT NOT NULL,
    session_id TEXT,
    memory_claim_id INTEGER,
    memory_id INTEGER,
    memory_type TEXT NOT NULL,
    topic_key TEXT,
    candidate_text_preview TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    FOREIGN KEY(memory_claim_id) REFERENCES memory_claims(id),
    FOREIGN KEY(memory_id) REFERENCES memories(id)
);

CREATE INDEX IF NOT EXISTS idx_memory_candidate_noops_claim
    ON memory_candidate_noops(memory_claim_id, created_at_epoch);

CREATE INDEX IF NOT EXISTS idx_memory_candidate_noops_project
    ON memory_candidate_noops(project, created_at_epoch);
