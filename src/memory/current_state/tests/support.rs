use anyhow::Result;
use rusqlite::{params, Connection};

use super::super::CurrentStateRequest;

pub(super) fn current_state_test_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_current_state_memory(
    conn: &Connection,
    id: i64,
    title: &str,
    content: &str,
    status: &str,
    state_key_id: i64,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
) -> Result<()> {
    insert_current_state_memory_at(
        conn,
        id,
        "/repo",
        title,
        content,
        status,
        state_key_id,
        1_700_000_000_i64 + id,
        valid_from_epoch,
        valid_to_epoch,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_current_state_memory_at(
    conn: &Connection,
    id: i64,
    project: &str,
    title: &str,
    content: &str,
    status: &str,
    state_key_id: i64,
    created_at_epoch: i64,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
) -> Result<()> {
    insert_current_state_memory_with_expiry_at(
        conn,
        id,
        project,
        title,
        content,
        status,
        state_key_id,
        created_at_epoch,
        None,
        valid_from_epoch,
        valid_to_epoch,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_current_state_memory_with_expiry_at(
    conn: &Connection,
    id: i64,
    project: &str,
    title: &str,
    content: &str,
    status: &str,
    state_key_id: i64,
    created_at_epoch: i64,
    expires_at_epoch: Option<i64>,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope, source_project,
          target_project, owner_scope, owner_key, context_class, expires_at_epoch, valid_from_epoch,
          valid_to_epoch, state_key_id)
         VALUES (?1, NULL, ?2, 'deploy-target', ?3, ?4, 'decision', NULL,
                 ?5, ?5, ?6, NULL, 'project', ?2, ?2, 'repo', ?2,
                 'startup_core', ?7, ?8, ?9, ?10)",
        params![
            id,
            project,
            title,
            content,
            created_at_epoch,
            status,
            expires_at_epoch,
            valid_from_epoch,
            valid_to_epoch,
            state_key_id
        ],
    )?;
    Ok(())
}

pub(super) fn insert_state_key_for(
    conn: &Connection,
    id: i64,
    owner_key: &str,
    current_memory_id: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label,
          state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'repo', ?2, 'decision', 'deploy-target',
                 'deploy target', 'active', ?3, 1, 10)",
        params![id, owner_key, current_memory_id],
    )?;
    Ok(())
}

pub(super) fn insert_state_key(conn: &Connection) -> Result<()> {
    insert_state_key_for(conn, 10, "/repo", None)
}

pub(super) fn set_current_memory(conn: &Connection, current_memory_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = ?1 WHERE id = 10",
        [current_memory_id],
    )?;
    Ok(())
}

pub(super) fn request() -> CurrentStateRequest {
    CurrentStateRequest {
        state_key: "deploy-target".to_string(),
        project: Some("/repo".to_string()),
        memory_type: Some("decision".to_string()),
        include_history: true,
        ..Default::default()
    }
}
