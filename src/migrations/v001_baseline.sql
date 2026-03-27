-- v001_baseline: Full schema as of remem v0.3.7 (schema version 13).
-- For NEW databases only. Existing databases (user_version >= 13) skip this.

-- === Core tables ===

CREATE TABLE IF NOT EXISTS sdk_sessions (
    id INTEGER PRIMARY KEY,
    content_session_id TEXT UNIQUE NOT NULL,
    memory_session_id TEXT NOT NULL,
    project TEXT,
    user_prompt TEXT,
    started_at TEXT,
    started_at_epoch INTEGER,
    status TEXT DEFAULT 'active',
    prompt_counter INTEGER DEFAULT 1
);

CREATE TABLE IF NOT EXISTS observations (
    id INTEGER PRIMARY KEY,
    memory_session_id TEXT NOT NULL,
    project TEXT,
    type TEXT NOT NULL,
    title TEXT,
    subtitle TEXT,
    narrative TEXT,
    facts TEXT,
    concepts TEXT,
    files_read TEXT,
    files_modified TEXT,
    prompt_number INTEGER,
    created_at TEXT,
    created_at_epoch INTEGER,
    discovery_tokens INTEGER DEFAULT 0,
    status TEXT DEFAULT 'active',
    last_accessed_epoch INTEGER,
    branch TEXT,
    commit_sha TEXT
);

CREATE TABLE IF NOT EXISTS session_summaries (
    id INTEGER PRIMARY KEY,
    memory_session_id TEXT NOT NULL,
    project TEXT,
    request TEXT,
    completed TEXT,
    decisions TEXT,
    learned TEXT,
    next_steps TEXT,
    preferences TEXT,
    prompt_number INTEGER,
    created_at TEXT,
    created_at_epoch INTEGER,
    discovery_tokens INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS pending_observations (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    project TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    tool_input TEXT,
    tool_response TEXT,
    cwd TEXT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_retry_epoch INTEGER,
    last_error TEXT,
    lease_owner TEXT,
    lease_expires_epoch INTEGER
);

CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY,
    session_id TEXT,
    project TEXT NOT NULL,
    topic_key TEXT,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    files TEXT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    branch TEXT,
    scope TEXT DEFAULT 'project'
);

CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    project TEXT NOT NULL,
    event_type TEXT NOT NULL,
    summary TEXT NOT NULL,
    detail TEXT,
    files TEXT,
    exit_code INTEGER,
    created_at_epoch INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS entities (
    id INTEGER PRIMARY KEY,
    canonical_name TEXT NOT NULL COLLATE NOCASE,
    entity_type TEXT,
    mention_count INTEGER DEFAULT 1,
    created_at_epoch INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    UNIQUE(canonical_name)
);

CREATE TABLE IF NOT EXISTS memory_entities (
    memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    entity_id INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    PRIMARY KEY(memory_id, entity_id)
);

CREATE TABLE IF NOT EXISTS summarize_cooldown (
    project TEXT PRIMARY KEY,
    last_summarize_epoch INTEGER NOT NULL,
    last_message_hash TEXT
);

CREATE TABLE IF NOT EXISTS summarize_locks (
    project TEXT PRIMARY KEY,
    lock_epoch INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS ai_usage_events (
    id INTEGER PRIMARY KEY,
    created_at TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    project TEXT,
    operation TEXT NOT NULL,
    executor TEXT NOT NULL,
    model TEXT,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    total_tokens INTEGER NOT NULL,
    estimated_cost_usd REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs (
    id INTEGER PRIMARY KEY,
    job_type TEXT NOT NULL,
    project TEXT NOT NULL,
    session_id TEXT,
    payload_json TEXT NOT NULL,
    state TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 6,
    lease_owner TEXT,
    lease_expires_epoch INTEGER,
    next_retry_epoch INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS workstreams (
    id INTEGER PRIMARY KEY,
    project TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    progress TEXT,
    next_action TEXT,
    blockers TEXT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    completed_at_epoch INTEGER
);

CREATE TABLE IF NOT EXISTS workstream_sessions (
    id INTEGER PRIMARY KEY,
    workstream_id INTEGER NOT NULL,
    memory_session_id TEXT NOT NULL,
    linked_at_epoch INTEGER NOT NULL,
    UNIQUE(workstream_id, memory_session_id)
);

-- === FTS (full-text search with trigram tokenizer for CJK) ===

CREATE VIRTUAL TABLE IF NOT EXISTS observations_fts USING fts5(
    title, subtitle, narrative, facts, concepts,
    content='observations',
    content_rowid='id',
    tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS observations_ai AFTER INSERT ON observations BEGIN
    INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
    VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
END;

CREATE TRIGGER IF NOT EXISTS observations_ad AFTER DELETE ON observations BEGIN
    INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
    VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
END;

CREATE TRIGGER IF NOT EXISTS observations_au AFTER UPDATE ON observations BEGIN
    INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
    VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
    INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
    VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
END;

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    title, content,
    content='memories',
    content_rowid='id',
    tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, title, content)
    VALUES (new.id, new.title, new.content);
END;

CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, content)
    VALUES ('delete', old.id, old.title, old.content);
END;

CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, content)
    VALUES ('delete', old.id, old.title, old.content);
    INSERT INTO memories_fts(rowid, title, content)
    VALUES (new.id, new.title, new.content);
END;

-- === Indexes ===

CREATE INDEX IF NOT EXISTS idx_observations_status ON observations(status);
CREATE INDEX IF NOT EXISTS idx_observations_project_status ON observations(project, status, created_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_observations_branch ON observations(project, branch, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_pending_session_lease ON pending_observations(session_id, lease_expires_epoch, id);
CREATE INDEX IF NOT EXISTS idx_pending_project_lease ON pending_observations(project, lease_expires_epoch, created_at_epoch);
CREATE INDEX IF NOT EXISTS idx_pending_claim_v2 ON pending_observations(status, session_id, next_retry_epoch, lease_expires_epoch, id);
CREATE INDEX IF NOT EXISTS idx_pending_project_v2 ON pending_observations(status, project, next_retry_epoch, created_at_epoch);

CREATE INDEX IF NOT EXISTS idx_sdk_sessions_msid ON sdk_sessions(memory_session_id);

CREATE INDEX IF NOT EXISTS idx_memories_project_status ON memories(project, status, updated_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_memories_topic_key ON memories(project, topic_key) WHERE topic_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(project, memory_type, updated_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_memories_branch ON memories(project, branch, updated_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope, status, updated_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id, created_at_epoch);
CREATE INDEX IF NOT EXISTS idx_events_project ON events(project, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_ai_usage_created ON ai_usage_events(created_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_ai_usage_project_created ON ai_usage_events(project, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_jobs_claim ON jobs(state, next_retry_epoch, priority, created_at_epoch, id);
CREATE INDEX IF NOT EXISTS idx_jobs_project_state ON jobs(project, state, created_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_lease ON jobs(state, lease_expires_epoch);

CREATE INDEX IF NOT EXISTS idx_workstreams_project_status ON workstreams(project, status, updated_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_workstream_sessions_ws ON workstream_sessions(workstream_id);
CREATE INDEX IF NOT EXISTS idx_workstream_sessions_session ON workstream_sessions(memory_session_id);

CREATE INDEX IF NOT EXISTS idx_session_summaries_project_created ON session_summaries(project, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_entity_name ON entities(canonical_name COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_memory_entities_entity ON memory_entities(entity_id);
