use anyhow::Result;
use rusqlite::Connection;

use crate::retrieval::memory_search::ProjectScopeFilter;

use super::super::extract::extract_entities;
use super::lookup::{query_memory_ids, search_by_query_words, search_by_query_words_with_scope};

pub fn search_by_entity(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<i64>> {
    search_by_entity_filtered(conn, query, project, None, None, limit, false)
}

pub fn search_by_entity_filtered(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
    include_inactive: bool,
) -> Result<Vec<i64>> {
    search_by_entity_filtered_with_scope(
        conn,
        query,
        project,
        memory_type,
        branch,
        limit,
        include_inactive,
        ProjectScopeFilter::IncludeGlobal,
    )
}

pub fn search_project_memories_by_entity_filtered(
    conn: &Connection,
    query: &str,
    project: &str,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
    include_inactive: bool,
) -> Result<Vec<i64>> {
    search_by_entity_filtered_with_scope(
        conn,
        query,
        Some(project),
        memory_type,
        branch,
        limit,
        include_inactive,
        ProjectScopeFilter::ProjectOnly,
    )
}

#[allow(clippy::too_many_arguments)]
fn search_by_entity_filtered_with_scope(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
    include_inactive: bool,
    scope_filter: ProjectScopeFilter,
) -> Result<Vec<i64>> {
    let query_entities = extract_entities(query, "");
    if query_entities.is_empty() {
        return match scope_filter {
            ProjectScopeFilter::IncludeGlobal => search_by_query_words(
                conn,
                query,
                project,
                memory_type,
                branch,
                limit,
                include_inactive,
            ),
            ProjectScopeFilter::ProjectOnly => search_by_query_words_with_scope(
                conn,
                query,
                project,
                memory_type,
                branch,
                limit,
                include_inactive,
                scope_filter,
            ),
        };
    }

    let mut all_ids = Vec::new();
    for entity_name in &query_entities {
        let ids = query_memory_ids(
            conn,
            "e.canonical_name = ?1 COLLATE NOCASE".to_string(),
            Box::new(entity_name.clone()),
            project,
            memory_type,
            branch,
            limit,
            include_inactive,
            scope_filter,
        )?;
        for id in ids {
            if !all_ids.contains(&id) {
                all_ids.push(id);
            }
        }
    }
    Ok(all_ids)
}
