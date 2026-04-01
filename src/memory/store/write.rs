use anyhow::Result;
use rusqlite::{params, Connection};

pub fn insert_memory(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
) -> Result<i64> {
    insert_memory_with_branch(
        conn,
        session_id,
        project,
        topic_key,
        title,
        content,
        memory_type,
        files,
        None,
    )
}

pub fn insert_memory_with_branch(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
) -> Result<i64> {
    insert_memory_full(
        conn,
        session_id,
        project,
        topic_key,
        title,
        content,
        memory_type,
        files,
        branch,
        "project",
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn insert_memory_full(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
    scope: &str,
    created_at_override: Option<i64>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let created_at = created_at_override.unwrap_or(now);

    if let Some(topic_key) = topic_key {
        if !topic_key.is_empty() {
            let existing_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM memories WHERE project = ?1 AND topic_key = ?2 LIMIT 1",
                    params![project, topic_key],
                    |row| row.get(0),
                )
                .ok();

            if let Some(id) = existing_id {
                conn.execute(
                    "UPDATE memories SET session_id = ?1, title = ?2, content = ?3, \
                     memory_type = ?4, files = ?5, updated_at_epoch = ?6, branch = ?7, \
                     scope = ?8 WHERE id = ?9",
                    params![session_id, title, content, memory_type, files, now, branch, scope, id],
                )?;
                refresh_memory_entities(conn, id, title, content, "entity link refresh failed");
                return Ok(id);
            }
        }
    }

    conn.execute(
        "INSERT INTO memories \
         (session_id, project, topic_key, title, content, memory_type, files, \
          created_at_epoch, updated_at_epoch, status, branch, scope) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, ?11)",
        params![
            session_id,
            project,
            topic_key,
            title,
            content,
            memory_type,
            files,
            created_at,
            now,
            branch,
            scope
        ],
    )?;
    let id = conn.last_insert_rowid();
    refresh_memory_entities(conn, id, title, content, "entity link failed");
    Ok(id)
}

fn refresh_memory_entities(conn: &Connection, id: i64, title: &str, content: &str, message: &str) {
    let entities = crate::entity::extract_entities(title, content);
    if entities.is_empty() {
        return;
    }
    if let Err(e) = crate::entity::link_entities(conn, id, &entities) {
        crate::log::warn("memory", &format!("{} for id={}: {}", message, id, e));
    }
}

#[cfg(test)]
mod tests;
