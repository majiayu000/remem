-- v006_capture_pipeline: append-only capture ledger + coalesced extraction tasks.
-- This is the first production slice of the memory pipeline. It keeps raw
-- events and task scheduling separate from pending_observations so high-frequency
-- tools cannot create one LLM job per event.

CREATE TABLE IF NOT EXISTS hosts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at_epoch INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS workspaces (
    id INTEGER PRIMARY KEY,
    root_path TEXT NOT NULL UNIQUE,
    git_remote TEXT,
    git_branch TEXT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
    id INTEGER PRIMARY KEY,
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_path TEXT NOT NULL,
    project_key TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(workspace_id, project_path)
);

CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_id TEXT NOT NULL,
    started_at_epoch INTEGER,
    last_seen_at_epoch INTEGER NOT NULL,
    status TEXT NOT NULL,
    UNIQUE(host_id, project_id, session_id)
);

CREATE TABLE IF NOT EXISTS event_blobs (
    id INTEGER PRIMARY KEY,
    content_hash TEXT NOT NULL UNIQUE,
    content_encoding TEXT NOT NULL,
    content_bytes BLOB NOT NULL,
    original_bytes INTEGER NOT NULL,
    stored_bytes INTEGER NOT NULL,
    created_at_epoch INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS captured_events (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER NOT NULL REFERENCES sessions(id),
    session_id TEXT NOT NULL,
    turn_id TEXT,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    role TEXT,
    tool_name TEXT,
    content_text TEXT,
    content_blob_id INTEGER REFERENCES event_blobs(id),
    content_hash TEXT NOT NULL,
    token_estimate INTEGER NOT NULL DEFAULT 0,
    retention_class TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    inserted_at_epoch INTEGER NOT NULL,
    UNIQUE(host_id, session_id, event_id)
);

CREATE TABLE IF NOT EXISTS extraction_tasks (
    id INTEGER PRIMARY KEY,
    task_kind TEXT NOT NULL,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER REFERENCES sessions(id),
    priority INTEGER NOT NULL,
    status TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    cursor_event_id INTEGER,
    high_watermark_event_id INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_retry_epoch INTEGER,
    lease_owner TEXT,
    lease_expires_epoch INTEGER,
    last_error TEXT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_candidates (
    id INTEGER PRIMARY KEY,
    project_id INTEGER REFERENCES projects(id),
    scope TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    topic_key TEXT NOT NULL,
    text TEXT NOT NULL,
    evidence_event_ids TEXT NOT NULL,
    confidence REAL NOT NULL,
    risk_class TEXT NOT NULL,
    review_status TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS rule_candidates (
    id INTEGER PRIMARY KEY,
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER REFERENCES projects(id),
    rule_path TEXT,
    rule_text TEXT NOT NULL,
    proposed_diff TEXT,
    evidence_event_ids TEXT NOT NULL,
    confidence REAL NOT NULL,
    review_status TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_host_project_seen
    ON sessions(host_id, project_id, last_seen_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_captured_events_session_event
    ON captured_events(session_row_id, event_id);
CREATE INDEX IF NOT EXISTS idx_captured_events_project_created
    ON captured_events(project_id, created_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_captured_events_event_type
    ON captured_events(event_type, retention_class);
CREATE INDEX IF NOT EXISTS idx_extraction_tasks_claim
    ON extraction_tasks(host_id, project_id, status, priority, next_retry_epoch, id);
CREATE INDEX IF NOT EXISTS idx_extraction_tasks_lease
    ON extraction_tasks(status, lease_expires_epoch);
CREATE INDEX IF NOT EXISTS idx_extraction_tasks_kind
    ON extraction_tasks(task_kind, status, created_at_epoch);
CREATE INDEX IF NOT EXISTS idx_memory_candidates_review
    ON memory_candidates(review_status, created_at_epoch);
CREATE INDEX IF NOT EXISTS idx_memory_candidates_project
    ON memory_candidates(project_id, scope, memory_type);
CREATE INDEX IF NOT EXISTS idx_rule_candidates_review
    ON rule_candidates(workspace_id, review_status);

INSERT OR IGNORE INTO hosts(name, enabled, created_at_epoch) VALUES
    ('claude-code', 1, strftime('%s', 'now')),
    ('codex-cli', 1, strftime('%s', 'now'));
