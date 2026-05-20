use crate::memory::Memory;
use crate::workstream::{WorkStream, WorkStreamStatus};
use rusqlite::{params, Connection};

mod load;
mod render;
mod sessions;

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
        "CREATE TABLE session_summaries (
            project TEXT,
            request TEXT,
            completed TEXT,
            created_at_epoch INTEGER
        );",
    )
    .unwrap();
}
