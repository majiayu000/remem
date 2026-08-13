use crate::memory::Memory;
use crate::workstream::{WorkStream, WorkStreamStatus};
use rusqlite::{params, Connection, OptionalExtension};

mod bundle_candidates;
mod codex_hook_stdout;
mod context_audit_persistence;
mod current_truth_activation;
mod cursor_hook;
mod diagnostics;
mod engine_convergence;
mod gate_pipeline;
mod load;
mod ownership;
mod project_alias;
mod render;
mod render_inline;
mod render_poisoning;
mod render_stability;
mod render_workstreams;
mod retrieval;
mod sessions;
mod staleness;
mod truncation;

pub(super) fn sample_memory(id: i64, memory_type: &str, title: &str) -> Memory {
    sample_memory_with_epoch(id, memory_type, title, 1_710_000_000)
}

pub(super) fn sample_memory_with_epoch(
    id: i64,
    memory_type: &str,
    title: &str,
    updated_at_epoch: i64,
) -> Memory {
    Memory {
        id,
        session_id: None,
        project: "demo/project".to_string(),
        topic_key: None,
        title: title.to_string(),
        text: "Body".to_string(),
        memory_type: memory_type.to_string(),
        files: None,
        created_at_epoch: updated_at_epoch,
        updated_at_epoch,
        status: "active".to_string(),
        branch: None,
        scope: "project".to_string(),
    }
}

pub(super) fn sample_workstream(id: i64, title: &str, next_action: Option<&str>) -> WorkStream {
    WorkStream {
        id,
        project: "demo/project".to_string(),
        title: title.to_string(),
        description: None,
        status: WorkStreamStatus::Active,
        progress: None,
        next_action: next_action.map(str::to_string),
        blockers: None,
        created_at_epoch: 0,
        updated_at_epoch: id,
        completed_at_epoch: None,
    }
}

pub(super) fn insert_memory(
    conn: &Connection,
    id: i64,
    project: &str,
    topic_key: Option<&str>,
    memory_type: &str,
    title: &str,
    content: &str,
    updated_at_epoch: i64,
) {
    insert_memory_with_branch(
        conn,
        id,
        project,
        topic_key,
        memory_type,
        title,
        content,
        updated_at_epoch,
        None,
    );
}

pub(super) fn insert_memory_with_branch(
    conn: &Connection,
    id: i64,
    project: &str,
    topic_key: Option<&str>,
    memory_type: &str,
    title: &str,
    content: &str,
    updated_at_epoch: i64,
    branch: Option<&str>,
) {
    let proof_event_id = 8_200_000 + id;
    let candidate_id = 7_200_000 + id;
    let state_key_id = 6_200_000 + id;
    let candidate_topic = topic_key.map_or_else(|| format!("context-fixture-{id}"), str::to_string);
    let has_capture_attribution: bool = conn
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM pragma_table_info('captured_events')
                 WHERE name = 'host_id'
             )",
            [],
            |row| row.get(0),
        )
        .unwrap();
    if has_capture_attribution {
        conn.execute_batch(
            "INSERT OR IGNORE INTO hosts (id, name, created_at_epoch)
             VALUES (9200001, 'context-fixture-host', 0);
             INSERT OR IGNORE INTO workspaces
             (id, root_path, created_at_epoch, updated_at_epoch)
             VALUES (9200001, '/context-fixture', 0, 0);
             INSERT OR IGNORE INTO projects
             (id, workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
             VALUES (9200001, 9200001, '/context-fixture', 'context-fixture', 0, 0);
             INSERT OR IGNORE INTO sessions
             (id, host_id, workspace_id, project_id, session_id, last_seen_at_epoch, status)
             VALUES (9200001, 9200001, 9200001, 9200001,
                     'context-fixture-session', 0, 'active');",
        )
        .unwrap();
        let parents: Option<(i64, i64, i64, i64)> = conn
            .query_row(
                "SELECT host.id, workspace.id, project.id, session.id
                 FROM hosts host
                 JOIN workspaces workspace ON workspace.id = 9200001
                 JOIN projects project ON project.workspace_id = workspace.id
                 JOIN sessions session ON session.project_id = project.id
                 WHERE host.id = session.host_id
                 LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .unwrap();
        let (host_id, workspace_id, project_id, session_row_id) =
            parents.expect("captured-event fixture parents");
        conn.execute(
            "INSERT INTO captured_events
             (id, host_id, workspace_id, project_id, session_row_id, session_id, event_id,
              event_type, content_hash, retention_class, created_at_epoch, inserted_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, 'context-fixture', ?6,
                     'message', ?7, 'normal', ?8, ?8)",
            params![
                proof_event_id,
                host_id,
                workspace_id,
                project_id,
                session_row_id,
                format!("context-proof-{id}"),
                format!("context-proof-hash-{id}"),
                updated_at_epoch
            ],
        )
        .unwrap();
    } else {
        conn.execute(
            "INSERT INTO captured_events (id) VALUES (?1)",
            [proof_event_id],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO memory_candidates
         (id, project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, NULL, 'project', ?2, ?3, ?4, ?5,
                 0.9, 'low', 'accepted', 0, 0)",
        params![
            candidate_id,
            memory_type,
            candidate_topic,
            content,
            format!("[{proof_event_id}]")
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key,
          created_at_epoch, updated_at_epoch)
         VALUES (?1, 'project', ?2, ?3, ?4, ?5, ?5)",
        params![
            state_key_id,
            project,
            memory_type,
            format!("context-fixture-{id}"),
            updated_at_epoch
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope,
          source_trust_class, source_candidate_id, state_key_id)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?7, 'active', ?8,
                 'project', 'local_tool_output', ?9, ?10)",
        params![
            id,
            project,
            topic_key,
            title,
            content,
            memory_type,
            updated_at_epoch,
            branch,
            candidate_id,
            state_key_id
        ],
    )
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_owned_memory(
    conn: &Connection,
    id: i64,
    project: &str,
    topic_key: Option<&str>,
    memory_type: &str,
    title: &str,
    content: &str,
    updated_at_epoch: i64,
    owner_scope: &str,
    owner_key: &str,
    target_project: Option<&str>,
    topic_domain: Option<&str>,
) {
    insert_memory(
        conn,
        id,
        project,
        topic_key,
        memory_type,
        title,
        content,
        updated_at_epoch,
    );
    conn.execute(
        "UPDATE memories
         SET source_project = ?1, target_project = ?2, owner_scope = ?3,
             owner_key = ?4, topic_domain = ?5, context_class = 'startup_core'
         WHERE id = ?6",
        params![
            project,
            target_project,
            owner_scope,
            owner_key,
            topic_domain,
            id
        ],
    )
    .unwrap();
}

pub(super) fn insert_global_memory(
    conn: &Connection,
    id: i64,
    project: &str,
    topic_key: Option<&str>,
    memory_type: &str,
    title: &str,
    content: &str,
    updated_at_epoch: i64,
) {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?7, 'active', NULL, 'global')",
        params![
            id,
            project,
            topic_key,
            title,
            content,
            memory_type,
            updated_at_epoch
        ],
    )
    .unwrap();
}

pub(super) fn insert_session_summary(
    conn: &Connection,
    project: &str,
    request: &str,
    completed: Option<&str>,
    created_at_epoch: i64,
) {
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            format!("session-{created_at_epoch}"),
            project,
            request,
            completed,
            created_at_epoch
        ],
    )
    .unwrap();
}

pub(super) fn create_session_summary_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_summaries (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT,
            project TEXT,
            request TEXT,
            completed TEXT,
            decisions TEXT,
            learned TEXT,
            next_steps TEXT,
            preferences TEXT,
            created_at_epoch INTEGER,
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
            session_row_id INTEGER,
            covered_from_event_id INTEGER,
            covered_to_event_id INTEGER,
            poisoning_status TEXT NOT NULL DEFAULT 'safe',
            quarantine_stage TEXT,
            quarantine_field TEXT,
            quarantine_event_id INTEGER,
            quarantine_pattern_id TEXT,
            quarantine_pattern_version INTEGER,
            acknowledged_pattern_id TEXT,
            acknowledged_pattern_version INTEGER,
            acknowledged_at_epoch INTEGER,
            poisoning_block_count INTEGER NOT NULL DEFAULT 0,
            poisoning_last_blocked_at_epoch INTEGER
        );
        CREATE TABLE IF NOT EXISTS sessions (
            id INTEGER PRIMARY KEY,
            session_id TEXT
        );",
    )
    .unwrap();
}

pub(super) fn create_workstream_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workstreams (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT,
            status TEXT NOT NULL,
            progress TEXT,
            next_action TEXT,
            blockers TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            completed_at_epoch INTEGER,
            owner_scope TEXT,
            owner_key TEXT,
            target_project TEXT,
            identity_key TEXT,
            merged_into_workstream_id INTEGER
        );",
    )
    .unwrap();
}

pub(super) fn setup_context_schema(conn: &Connection) {
    crate::memory::types::tests_helper::setup_memory_schema(conn);
    create_session_summary_schema(conn);
    create_workstream_schema(conn);
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS graph_edges (
            id INTEGER PRIMARY KEY,
            edge_type TEXT NOT NULL,
            edge_trust TEXT,
            from_node_kind TEXT,
            from_node_id INTEGER,
            to_node_kind TEXT,
            to_node_id INTEGER,
            created_at_epoch INTEGER NOT NULL,
            valid_from_epoch INTEGER,
            valid_to_epoch INTEGER
        );",
    )
    .unwrap();
}
