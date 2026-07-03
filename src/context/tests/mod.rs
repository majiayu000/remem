use crate::memory::Memory;
use crate::workstream::{WorkStream, WorkStreamStatus};
use rusqlite::{params, Connection};

mod codex_hook_stdout;
mod diagnostics;
mod gate_pipeline;
mod load;
mod ownership;
mod render;
mod render_stability;
mod retrieval;
mod sessions;
mod staleness;

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
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?7, 'active', ?8, 'project')",
        params![
            id,
            project,
            topic_key,
            title,
            content,
            memory_type,
            updated_at_epoch,
            branch
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
        "INSERT INTO session_summaries (project, request, completed, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4)",
        params![project, request, completed, created_at_epoch],
    )
    .unwrap();
}

pub(super) fn create_session_summary_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_summaries (
            project TEXT,
            request TEXT,
            completed TEXT,
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
            covered_to_event_id INTEGER
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
}
