-- schema_001_baseline: full memory-system schema for a fresh ~/.remem/schema.sqlite
-- file. The existing remem.db file is untouched.

PRAGMA foreign_keys = ON;

-- === Identity tables (§6.1-6.4) ===

CREATE TABLE IF NOT EXISTS hosts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,                  -- claude-code | codex-cli
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
    status TEXT NOT NULL,                       -- active | stopped | abandoned
    UNIQUE(host_id, project_id, session_id)
);

-- === Capture layer (§6.5-6.6) ===

CREATE TABLE IF NOT EXISTS event_blobs (
    id INTEGER PRIMARY KEY,
    content_hash TEXT NOT NULL UNIQUE,
    content_encoding TEXT NOT NULL,             -- plain | gzip
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
    event_id TEXT NOT NULL,                     -- remem-synthesized
    event_type TEXT NOT NULL,                   -- user_message|assistant_message|tool_call|tool_result|file_edit|session_stop
    role TEXT,
    tool_name TEXT,
    content_text TEXT,                          -- D1: <=16 KiB direct, else prefix/suffix + digest
    content_blob_id INTEGER REFERENCES event_blobs(id),
    content_hash TEXT NOT NULL,
    token_estimate INTEGER NOT NULL DEFAULT 0,
    retention_class TEXT NOT NULL,              -- raw_keep | raw_compact | raw_drop_candidate | truncated
    created_at_epoch INTEGER NOT NULL,
    inserted_at_epoch INTEGER NOT NULL,
    UNIQUE(host_id, session_id, event_id)
);

-- === Extraction queue (§6.7) ===

CREATE TABLE IF NOT EXISTS extraction_tasks (
    id INTEGER PRIMARY KEY,
    task_kind TEXT NOT NULL,                    -- session_rollup|observation_extract|memory_candidate|rule_candidate|index_update
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER REFERENCES sessions(id),
    priority INTEGER NOT NULL,
    status TEXT NOT NULL,                       -- pending|processing|delayed|done|failed
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

-- === Derived knowledge (§6.8-6.9) ===

CREATE TABLE IF NOT EXISTS session_summaries (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER NOT NULL REFERENCES sessions(id),
    summary_text TEXT NOT NULL,
    covered_from_event_id INTEGER NOT NULL,
    covered_to_event_id INTEGER NOT NULL,
    model TEXT,
    created_at_epoch INTEGER NOT NULL,
    UNIQUE(session_row_id, covered_from_event_id, covered_to_event_id)
);

CREATE TABLE IF NOT EXISTS observations (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER NOT NULL REFERENCES sessions(id),
    observation_type TEXT NOT NULL,             -- action|discovery|error|decision_hint|preference_hint
    text TEXT NOT NULL,
    evidence_event_ids TEXT NOT NULL,           -- JSON array of captured_events ids
    confidence REAL NOT NULL,
    created_at_epoch INTEGER NOT NULL
);

-- === Curated memory (§6.10-6.12) ===

CREATE TABLE IF NOT EXISTS memory_candidates (
    id INTEGER PRIMARY KEY,
    project_id INTEGER REFERENCES projects(id),
    scope TEXT NOT NULL,                        -- global|workspace|project
    memory_type TEXT NOT NULL,                  -- decision|discovery|bugfix|architecture|preference
    topic_key TEXT NOT NULL,
    text TEXT NOT NULL,
    evidence_event_ids TEXT NOT NULL,
    confidence REAL NOT NULL,
    risk_class TEXT NOT NULL,                   -- low|medium|high
    review_status TEXT NOT NULL,                -- auto_promoted|pending_review|approved|edited|discarded
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY,
    project_id INTEGER REFERENCES projects(id),
    scope TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    topic_key TEXT NOT NULL,
    text TEXT NOT NULL,
    evidence_event_ids TEXT NOT NULL,
    source_candidate_id INTEGER REFERENCES memory_candidates(id),
    confidence REAL NOT NULL,
    status TEXT NOT NULL,                       -- active|stale|superseded|rejected
    stale_after_epoch INTEGER,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS rule_candidates (
    id INTEGER PRIMARY KEY,
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER REFERENCES projects(id),
    rule_path TEXT,
    rule_text TEXT NOT NULL,
    proposed_diff TEXT,                         -- D4: optional unified diff, plain text is primary
    evidence_event_ids TEXT NOT NULL,
    confidence REAL NOT NULL,
    review_status TEXT NOT NULL,                -- pending_review|approved|discarded|exported
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

-- === Operational ===

CREATE TABLE IF NOT EXISTS worker_heartbeats (
    owner TEXT PRIMARY KEY,
    pid INTEGER,
    mode TEXT NOT NULL,                         -- daemon | once
    started_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

-- === Indexes ===

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

CREATE INDEX IF NOT EXISTS idx_observations_session
    ON observations(session_row_id, created_at_epoch);
CREATE INDEX IF NOT EXISTS idx_observations_project_type
    ON observations(project_id, observation_type);

CREATE INDEX IF NOT EXISTS idx_memory_candidates_review
    ON memory_candidates(review_status, created_at_epoch);
CREATE INDEX IF NOT EXISTS idx_memory_candidates_project
    ON memory_candidates(project_id, scope, memory_type);

CREATE INDEX IF NOT EXISTS idx_memories_project_status
    ON memories(project_id, scope, status, updated_at_epoch DESC);
-- §6.11 UNIQUE(scope, COALESCE(project_id, 0), topic_key) needs an expression
-- index, since SQLite UNIQUE table constraints reject expressions.
CREATE UNIQUE INDEX IF NOT EXISTS idx_memories_topic_unique
    ON memories(scope, COALESCE(project_id, 0), topic_key);

CREATE INDEX IF NOT EXISTS idx_rule_candidates_review
    ON rule_candidates(workspace_id, review_status);

CREATE INDEX IF NOT EXISTS idx_session_summaries_session
    ON session_summaries(session_row_id, covered_from_event_id);

-- === FTS (D2: vector attaches to memories only; FTS still useful) ===

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    text,
    content='memories',
    content_rowid='id',
    tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, text) VALUES (new.id, new.text);
END;

CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO memories_fts(rowid, text) VALUES (new.id, new.text);
END;

-- === Seed hosts ===

INSERT OR IGNORE INTO hosts(name, enabled, created_at_epoch) VALUES
    ('claude-code', 1, strftime('%s', 'now')),
    ('codex-cli', 1, strftime('%s', 'now'));
