-- v070_web_console_governance: stable Web mutation versions, replay ledger,
-- and never-reused cursor source identifiers.

-- SQLite validates trigger bodies while their referenced tables are rebuilt.
-- Remove every trigger that depends on a rebuilt table before any ALTER/DROP;
-- the canonical definitions are restored below in the same transaction.
DROP TRIGGER IF EXISTS observations_ai;
DROP TRIGGER IF EXISTS observations_ad;
DROP TRIGGER IF EXISTS observations_au;
DROP TRIGGER IF EXISTS memories_au;
DROP TRIGGER IF EXISTS graph_edges_validate_source_events_insert;
DROP TRIGGER IF EXISTS graph_edges_validate_source_events_update;
DROP TRIGGER IF EXISTS graph_edges_validate_nodes_insert;
DROP TRIGGER IF EXISTS graph_edges_validate_nodes_update;

ALTER TABLE memory_candidates ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE memories ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE memories ADD COLUMN web_archive_operation_id TEXT;

CREATE TABLE api_mutation_requests (
    idempotency_key_hash TEXT PRIMARY KEY,
    request_hash TEXT NOT NULL,
    operation_id TEXT NOT NULL UNIQUE,
    resource_kind TEXT NOT NULL,
    resource_id INTEGER NOT NULL,
    action TEXT NOT NULL,
    response_schema_version INTEGER NOT NULL,
    response_json TEXT NOT NULL,
    audit_id INTEGER NOT NULL,
    created_at_epoch INTEGER NOT NULL
);

CREATE INDEX idx_api_mutation_requests_resource
    ON api_mutation_requests(resource_kind, resource_id, action, created_at_epoch DESC);

-- These five tables back opaque keyset cursors. Rebuild them with
-- AUTOINCREMENT while foreign_keys is disabled by the v070 migration protocol.
CREATE TABLE _v070_observations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
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
    commit_sha TEXT,
    host_id INTEGER,
    project_id INTEGER,
    session_row_id INTEGER,
    observation_type TEXT,
    text TEXT,
    evidence_event_ids TEXT,
    confidence REAL,
    reference_time_epoch INTEGER
);

INSERT INTO _v070_observations (
    id, memory_session_id, project, type, title, subtitle, narrative, facts,
    concepts, files_read, files_modified, prompt_number, created_at,
    created_at_epoch, discovery_tokens, status, last_accessed_epoch, branch,
    commit_sha, host_id, project_id, session_row_id, observation_type, text,
    evidence_event_ids, confidence, reference_time_epoch
)
SELECT
    id, memory_session_id, project, type, title, subtitle, narrative, facts,
    concepts, files_read, files_modified, prompt_number, created_at,
    created_at_epoch, discovery_tokens, status, last_accessed_epoch, branch,
    commit_sha, host_id, project_id, session_row_id, observation_type, text,
    evidence_event_ids, confidence, reference_time_epoch
FROM observations;

DROP TABLE observations;
ALTER TABLE _v070_observations RENAME TO observations;

CREATE INDEX idx_observations_status ON observations(status);
CREATE INDEX idx_observations_project_status
    ON observations(project, status, created_at_epoch DESC);
CREATE INDEX idx_observations_branch
    ON observations(project, branch, created_at_epoch DESC);
CREATE INDEX idx_observations_session_evidence
    ON observations(session_row_id, evidence_event_ids);
CREATE INDEX idx_observations_project_type
    ON observations(project_id, observation_type, created_at_epoch DESC);
CREATE INDEX idx_observations_project_reference_time
    ON observations(project_id, reference_time_epoch DESC);

CREATE TRIGGER observations_ai AFTER INSERT ON observations BEGIN
    INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
    VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
END;

CREATE TRIGGER observations_ad AFTER DELETE ON observations BEGIN
    INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
    VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
END;

CREATE TRIGGER observations_au AFTER UPDATE ON observations BEGIN
    INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
    VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
    INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
    VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
END;

CREATE TABLE _v070_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_id TEXT NOT NULL,
    started_at_epoch INTEGER,
    last_seen_at_epoch INTEGER NOT NULL,
    status TEXT NOT NULL,
    UNIQUE(host_id, project_id, session_id)
);

INSERT INTO _v070_sessions (
    id, host_id, workspace_id, project_id, session_id, started_at_epoch,
    last_seen_at_epoch, status
)
SELECT
    id, host_id, workspace_id, project_id, session_id, started_at_epoch,
    last_seen_at_epoch, status
FROM sessions;

DROP TABLE sessions;
ALTER TABLE _v070_sessions RENAME TO sessions;

CREATE INDEX idx_sessions_host_project_seen
    ON sessions(host_id, project_id, last_seen_at_epoch DESC);

CREATE TABLE _v070_workstreams (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    progress TEXT,
    next_action TEXT,
    blockers TEXT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    completed_at_epoch INTEGER,
    source_project TEXT,
    target_project TEXT,
    owner_scope TEXT,
    owner_key TEXT,
    topic_domain TEXT,
    routing_confidence REAL,
    routing_reason TEXT,
    context_class TEXT,
    expires_at_epoch INTEGER,
    valid_from_epoch INTEGER,
    valid_to_epoch INTEGER,
    identity_key TEXT,
    merged_into_workstream_id INTEGER REFERENCES workstreams(id)
);

INSERT INTO _v070_workstreams (
    id, project, title, description, status, progress, next_action, blockers,
    created_at_epoch, updated_at_epoch, completed_at_epoch, source_project,
    target_project, owner_scope, owner_key, topic_domain, routing_confidence,
    routing_reason, context_class, expires_at_epoch, valid_from_epoch,
    valid_to_epoch, identity_key, merged_into_workstream_id
)
SELECT
    id, project, title, description, status, progress, next_action, blockers,
    created_at_epoch, updated_at_epoch, completed_at_epoch, source_project,
    target_project, owner_scope, owner_key, topic_domain, routing_confidence,
    routing_reason, context_class, expires_at_epoch, valid_from_epoch,
    valid_to_epoch, identity_key, merged_into_workstream_id
FROM workstreams;

DROP TABLE workstreams;
ALTER TABLE _v070_workstreams RENAME TO workstreams;

CREATE INDEX idx_workstreams_project_status
    ON workstreams(project, status, updated_at_epoch DESC);
CREATE INDEX idx_workstreams_owner_status
    ON workstreams(owner_scope, owner_key, status, updated_at_epoch DESC);
CREATE INDEX idx_workstreams_target_status
    ON workstreams(target_project, status, updated_at_epoch DESC);
CREATE INDEX idx_workstreams_identity_key ON workstreams(identity_key);
CREATE INDEX idx_workstreams_merged_into ON workstreams(merged_into_workstream_id);

CREATE TABLE _v070_captured_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
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
    reference_time_epoch INTEGER,
    UNIQUE(host_id, session_id, event_id)
);

INSERT INTO _v070_captured_events (
    id, host_id, workspace_id, project_id, session_row_id, session_id, turn_id,
    event_id, event_type, role, tool_name, content_text, content_blob_id,
    content_hash, token_estimate, retention_class, created_at_epoch,
    inserted_at_epoch, reference_time_epoch
)
SELECT
    id, host_id, workspace_id, project_id, session_row_id, session_id, turn_id,
    event_id, event_type, role, tool_name, content_text, content_blob_id,
    content_hash, token_estimate, retention_class, created_at_epoch,
    inserted_at_epoch, reference_time_epoch
FROM captured_events;

DROP TABLE captured_events;
ALTER TABLE _v070_captured_events RENAME TO captured_events;

CREATE INDEX idx_captured_events_session_event
    ON captured_events(session_row_id, event_id);
CREATE INDEX idx_captured_events_project_created
    ON captured_events(project_id, created_at_epoch DESC);
CREATE INDEX idx_captured_events_event_type
    ON captured_events(event_type, retention_class);
CREATE INDEX idx_captured_events_project_reference_time
    ON captured_events(project_id, reference_time_epoch DESC);

CREATE TRIGGER graph_edges_captured_events_delete
AFTER DELETE ON captured_events
BEGIN
    DELETE FROM graph_edges
    WHERE (from_node_kind = 'episode' AND from_node_id = OLD.id)
       OR (to_node_kind = 'episode' AND to_node_id = OLD.id)
       OR EXISTS (
           SELECT 1
           FROM json_each(graph_edges.source_event_ids) AS source_event
           WHERE source_event.value = OLD.id
       );
END;

CREATE TRIGGER graph_edges_validate_source_events_insert
BEFORE INSERT ON graph_edges
WHEN NEW.edge_trust = 'trusted'
BEGIN
    SELECT CASE
        WHEN EXISTS (
            SELECT 1
            FROM json_each(NEW.source_event_ids) AS source_event
            WHERE source_event.type != 'integer'
               OR source_event.value <= 0
        )
        THEN RAISE(ABORT, 'graph_edges trusted source_event_ids must be positive integers')
    END;

    SELECT CASE
        WHEN EXISTS (
            SELECT 1
            FROM json_each(NEW.source_event_ids) AS source_event
            WHERE NOT EXISTS (
                SELECT 1 FROM captured_events WHERE id = source_event.value
            )
        )
        THEN RAISE(ABORT, 'graph_edges trusted source event missing')
    END;
END;

CREATE TRIGGER graph_edges_validate_source_events_update
BEFORE UPDATE OF edge_trust, source_event_ids ON graph_edges
WHEN NEW.edge_trust = 'trusted'
BEGIN
    SELECT CASE
        WHEN EXISTS (
            SELECT 1
            FROM json_each(NEW.source_event_ids) AS source_event
            WHERE source_event.type != 'integer'
               OR source_event.value <= 0
        )
        THEN RAISE(ABORT, 'graph_edges trusted source_event_ids must be positive integers')
    END;

    SELECT CASE
        WHEN EXISTS (
            SELECT 1
            FROM json_each(NEW.source_event_ids) AS source_event
            WHERE NOT EXISTS (
                SELECT 1 FROM captured_events WHERE id = source_event.value
            )
        )
        THEN RAISE(ABORT, 'graph_edges trusted source event missing')
    END;
END;

CREATE TRIGGER graph_edges_validate_nodes_insert
BEFORE INSERT ON graph_edges
BEGIN
    SELECT CASE
        WHEN NEW.from_node_kind = 'memory'
             AND NOT EXISTS (SELECT 1 FROM memories WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from memory node missing')
        WHEN NEW.from_node_kind = 'entity'
             AND NOT EXISTS (SELECT 1 FROM entities WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from entity node missing')
        WHEN NEW.from_node_kind = 'fact'
             AND NOT EXISTS (SELECT 1 FROM memory_facts WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from fact node missing')
        WHEN NEW.from_node_kind = 'episode'
             AND NOT EXISTS (SELECT 1 FROM captured_events WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from episode node missing')
        WHEN NEW.from_node_kind = 'state'
             AND NOT EXISTS (SELECT 1 FROM memory_state_keys WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from state node missing')
        WHEN NEW.from_node_kind = 'topic'
             AND NOT EXISTS (SELECT 1 FROM topic_segments WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from topic node missing')
        WHEN NEW.from_node_kind = 'file'
             AND NOT EXISTS (SELECT 1 FROM graph_file_nodes WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from file node missing')
    END;

    SELECT CASE
        WHEN NEW.to_node_kind = 'memory'
             AND NOT EXISTS (SELECT 1 FROM memories WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to memory node missing')
        WHEN NEW.to_node_kind = 'entity'
             AND NOT EXISTS (SELECT 1 FROM entities WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to entity node missing')
        WHEN NEW.to_node_kind = 'fact'
             AND NOT EXISTS (SELECT 1 FROM memory_facts WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to fact node missing')
        WHEN NEW.to_node_kind = 'episode'
             AND NOT EXISTS (SELECT 1 FROM captured_events WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to episode node missing')
        WHEN NEW.to_node_kind = 'state'
             AND NOT EXISTS (SELECT 1 FROM memory_state_keys WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to state node missing')
        WHEN NEW.to_node_kind = 'topic'
             AND NOT EXISTS (SELECT 1 FROM topic_segments WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to topic node missing')
        WHEN NEW.to_node_kind = 'file'
             AND NOT EXISTS (SELECT 1 FROM graph_file_nodes WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to file node missing')
    END;
END;

CREATE TRIGGER graph_edges_validate_nodes_update
BEFORE UPDATE OF from_node_kind, from_node_id, to_node_kind, to_node_id ON graph_edges
BEGIN
    SELECT CASE
        WHEN NEW.from_node_kind = 'memory'
             AND NOT EXISTS (SELECT 1 FROM memories WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from memory node missing')
        WHEN NEW.from_node_kind = 'entity'
             AND NOT EXISTS (SELECT 1 FROM entities WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from entity node missing')
        WHEN NEW.from_node_kind = 'fact'
             AND NOT EXISTS (SELECT 1 FROM memory_facts WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from fact node missing')
        WHEN NEW.from_node_kind = 'episode'
             AND NOT EXISTS (SELECT 1 FROM captured_events WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from episode node missing')
        WHEN NEW.from_node_kind = 'state'
             AND NOT EXISTS (SELECT 1 FROM memory_state_keys WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from state node missing')
        WHEN NEW.from_node_kind = 'topic'
             AND NOT EXISTS (SELECT 1 FROM topic_segments WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from topic node missing')
        WHEN NEW.from_node_kind = 'file'
             AND NOT EXISTS (SELECT 1 FROM graph_file_nodes WHERE id = NEW.from_node_id)
        THEN RAISE(ABORT, 'graph_edges from file node missing')
    END;

    SELECT CASE
        WHEN NEW.to_node_kind = 'memory'
             AND NOT EXISTS (SELECT 1 FROM memories WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to memory node missing')
        WHEN NEW.to_node_kind = 'entity'
             AND NOT EXISTS (SELECT 1 FROM entities WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to entity node missing')
        WHEN NEW.to_node_kind = 'fact'
             AND NOT EXISTS (SELECT 1 FROM memory_facts WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to fact node missing')
        WHEN NEW.to_node_kind = 'episode'
             AND NOT EXISTS (SELECT 1 FROM captured_events WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to episode node missing')
        WHEN NEW.to_node_kind = 'state'
             AND NOT EXISTS (SELECT 1 FROM memory_state_keys WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to state node missing')
        WHEN NEW.to_node_kind = 'topic'
             AND NOT EXISTS (SELECT 1 FROM topic_segments WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to topic node missing')
        WHEN NEW.to_node_kind = 'file'
             AND NOT EXISTS (SELECT 1 FROM graph_file_nodes WHERE id = NEW.to_node_id)
        THEN RAISE(ABORT, 'graph_edges to file node missing')
    END;
END;

CREATE TABLE _v070_extraction_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
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
    updated_at_epoch INTEGER NOT NULL,
    replay_range_id INTEGER REFERENCES extraction_replay_ranges(id),
    failure_class TEXT,
    failed_at_epoch INTEGER,
    archived_at_epoch INTEGER
);

INSERT INTO _v070_extraction_tasks (
    id, task_kind, host_id, workspace_id, project_id, session_row_id, priority,
    status, idempotency_key, cursor_event_id, high_watermark_event_id, attempts,
    next_retry_epoch, lease_owner, lease_expires_epoch, last_error,
    created_at_epoch, updated_at_epoch, replay_range_id, failure_class,
    failed_at_epoch, archived_at_epoch
)
SELECT
    id, task_kind, host_id, workspace_id, project_id, session_row_id, priority,
    status, idempotency_key, cursor_event_id, high_watermark_event_id, attempts,
    next_retry_epoch, lease_owner, lease_expires_epoch, last_error,
    created_at_epoch, updated_at_epoch, replay_range_id, failure_class,
    failed_at_epoch, archived_at_epoch
FROM extraction_tasks;

DROP TABLE extraction_tasks;
ALTER TABLE _v070_extraction_tasks RENAME TO extraction_tasks;

CREATE INDEX idx_extraction_tasks_claim
    ON extraction_tasks(host_id, project_id, status, priority, next_retry_epoch, id);
CREATE INDEX idx_extraction_tasks_lease
    ON extraction_tasks(status, lease_expires_epoch);
CREATE INDEX idx_extraction_tasks_kind
    ON extraction_tasks(task_kind, status, created_at_epoch);
CREATE INDEX idx_extraction_tasks_replay_range
    ON extraction_tasks(replay_range_id, task_kind, status);
CREATE INDEX idx_extraction_tasks_failure_lifecycle
    ON extraction_tasks(status, archived_at_epoch, failed_at_epoch, failure_class);

DELETE FROM sqlite_sequence
WHERE name IN ('observations', 'sessions', 'workstreams', 'captured_events', 'extraction_tasks');
INSERT INTO sqlite_sequence(name, seq)
    SELECT 'observations', COALESCE(MAX(id), 0) FROM observations;
INSERT INTO sqlite_sequence(name, seq)
    SELECT 'sessions', COALESCE(MAX(id), 0) FROM sessions;
INSERT INTO sqlite_sequence(name, seq)
    SELECT 'workstreams', COALESCE(MAX(id), 0) FROM workstreams;
INSERT INTO sqlite_sequence(name, seq)
    SELECT 'captured_events', COALESCE(MAX(id), 0) FROM captured_events;
INSERT INTO sqlite_sequence(name, seq)
    SELECT 'extraction_tasks', COALESCE(MAX(id), 0) FROM extraction_tasks;

-- The version bump below performs an internal UPDATE. Restrict the FTS update
-- trigger to indexed fields so that internal metadata writes cannot issue a
-- second external-content delete and corrupt the FTS index.
CREATE TRIGGER memories_au
AFTER UPDATE OF title, content, search_context ON memories
BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, content, search_context)
    VALUES ('delete', old.id, old.title, old.content, COALESCE(old.search_context, ''));
    INSERT INTO memories_fts(rowid, title, content, search_context)
    VALUES (new.id, new.title, new.content, COALESCE(new.search_context, ''));
END;

CREATE TRIGGER memory_candidates_web_version
AFTER UPDATE OF
    project_id, scope, memory_type, topic_key, text, evidence_event_ids,
    confidence, risk_class, review_status, updated_at_epoch, source_project,
    target_project, owner_scope, owner_key, topic_domain, routing_confidence,
    routing_reason, context_class, expires_at_epoch, valid_from_epoch,
    valid_to_epoch, state_key, state_key_confidence, state_key_reason,
    auto_promote_block_reason, source_kind, review_actor, reviewed_at_epoch,
    review_action_source, review_batch_id, review_reason, source_trust_class,
    quarantine_pattern_id, quarantine_pattern_version, acknowledged_pattern_id,
    acknowledged_pattern_version, acknowledged_at_epoch
ON memory_candidates
BEGIN
    UPDATE memory_candidates SET version = version + 1 WHERE id = NEW.id;
END;

CREATE TRIGGER memories_web_version
AFTER UPDATE OF
    session_id, project, topic_key, title, content, memory_type, files,
    updated_at_epoch, status, branch, scope, evidence_event_ids,
    source_candidate_id, confidence, search_context, source_project,
    target_project, owner_scope, owner_key, topic_domain, routing_confidence,
    routing_reason, context_class, expires_at_epoch, valid_from_epoch,
    valid_to_epoch, state_key_id, reference_time_epoch, source_trust_class,
    acknowledged_pattern_id, acknowledged_pattern_version,
    acknowledged_at_epoch
ON memories
BEGIN
    UPDATE memories SET version = version + 1 WHERE id = NEW.id;
END;

CREATE TRIGGER memories_clear_web_archive_marker
AFTER UPDATE OF status ON memories
WHEN OLD.status IS NOT NEW.status
BEGIN
    UPDATE memories SET web_archive_operation_id = NULL WHERE id = NEW.id;
END;
