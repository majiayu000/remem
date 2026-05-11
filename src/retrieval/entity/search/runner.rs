use anyhow::Result;
use rusqlite::Connection;

use super::super::extract::extract_entities;
use super::lookup::{query_memory_ids, search_by_query_words};

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
    let query_entities = extract_entities(query, "");
    if query_entities.is_empty() {
        return search_by_query_words(
            conn,
            query,
            project,
            memory_type,
            branch,
            limit,
            include_inactive,
        );
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
        )?;
        for id in ids {
            if !all_ids.contains(&id) {
                all_ids.push(id);
            }
        }
    }
    Ok(all_ids)
}
