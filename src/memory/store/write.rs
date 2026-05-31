use anyhow::Result;
use rusqlite::{params, Connection};

use crate::memory::search_context::build_search_context;

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
    let search_context = build_search_context(memory_type, topic_key, content, files);
    let ownership = default_ownership(project, scope);

    if let Some(topic_key) = topic_key {
        if !topic_key.is_empty() {
            let existing_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM memories
                     WHERE project = ?1 AND topic_key = ?2 AND scope = ?3
                     LIMIT 1",
                    params![project, topic_key, scope],
                    |row| row.get(0),
                )
                .ok();

            if let Some(id) = existing_id {
                conn.execute(
                    "UPDATE memories SET session_id = ?1, title = ?2, content = ?3, \
                     memory_type = ?4, files = ?5, updated_at_epoch = ?6, branch = ?7, \
                     scope = ?8, search_context = ?9, \
                     source_project = COALESCE(source_project, ?10), \
                     target_project = COALESCE(target_project, ?11), \
                     owner_scope = COALESCE(owner_scope, ?12), \
                     owner_key = COALESCE(owner_key, ?13), \
                     context_class = COALESCE(context_class, ?14) \
                     WHERE id = ?15",
                    params![
                        session_id,
                        title,
                        content,
                        memory_type,
                        files,
                        now,
                        branch,
                        scope,
                        search_context,
                        ownership.source_project,
                        ownership.target_project,
                        ownership.owner_scope,
                        ownership.owner_key,
                        ownership.context_class,
                        id
                    ],
                )?;
                refresh_memory_entities(conn, id, title, content, "entity link refresh failed");
                return Ok(id);
            }
        }
    }

    conn.execute(
        "INSERT INTO memories \
         (session_id, project, topic_key, title, content, memory_type, files, search_context, \
          created_at_epoch, updated_at_epoch, status, branch, scope, \
          source_project, target_project, owner_scope, owner_key, context_class) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active', ?11, ?12, \
                 ?13, ?14, ?15, ?16, ?17)",
        params![
            session_id,
            project,
            topic_key,
            title,
            content,
            memory_type,
            files,
            search_context,
            created_at,
            now,
            branch,
            scope,
            ownership.source_project,
            ownership.target_project,
            ownership.owner_scope,
            ownership.owner_key,
            ownership.context_class
        ],
    )?;
    let id = conn.last_insert_rowid();
    refresh_memory_entities(conn, id, title, content, "entity link failed");
    Ok(id)
}

struct DefaultOwnership<'a> {
    source_project: &'a str,
    target_project: Option<&'a str>,
    owner_scope: &'static str,
    owner_key: &'a str,
    context_class: &'static str,
}

fn default_ownership<'a>(project: &'a str, scope: &str) -> DefaultOwnership<'a> {
    if scope == "global" {
        DefaultOwnership {
            source_project: project,
            target_project: None,
            owner_scope: "user",
            owner_key: "user:default",
            context_class: "startup_core",
        }
    } else {
        DefaultOwnership {
            source_project: project,
            target_project: Some(project),
            owner_scope: "repo",
            owner_key: project,
            context_class: "startup_core",
        }
    }
}

fn refresh_memory_entities(conn: &Connection, id: i64, title: &str, content: &str, message: &str) {
    let entities = crate::retrieval::entity::extract_entities(title, content);
    if entities.is_empty() {
        return;
    }
    if let Err(e) = crate::retrieval::entity::link_entities(conn, id, &entities) {
        crate::log::warn("memory", &format!("{} for id={}: {}", message, id, e));
    }
}

#[cfg(test)]
mod tests;
