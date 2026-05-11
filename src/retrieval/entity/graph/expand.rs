use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use super::seed::load_seed_entity_ids;
use super::sql::{build_expand_params, build_expand_sql};

pub fn expand_via_entity_graph(
    conn: &Connection,
    seed_memory_ids: &[i64],
    exclude_ids: &[i64],
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<i64>> {
    expand_via_entity_graph_filtered(
        conn,
        seed_memory_ids,
        exclude_ids,
        project,
        None,
        None,
        limit,
        false,
    )
}

pub fn expand_via_entity_graph_filtered(
    conn: &Connection,
    seed_memory_ids: &[i64],
    exclude_ids: &[i64],
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
    include_inactive: bool,
) -> Result<Vec<i64>> {
    if seed_memory_ids.is_empty() {
        return Ok(vec![]);
    }

    let entity_ids = load_seed_entity_ids(conn, seed_memory_ids)?;
    if entity_ids.is_empty() {
        return Ok(vec![]);
    }

    let exclude_set: HashSet<i64> = exclude_ids.iter().copied().collect();
    let seed_set: HashSet<i64> = seed_memory_ids.iter().copied().collect();
    let sql = build_expand_sql(
        entity_ids.len(),
        project,
        memory_type,
        branch,
        include_inactive,
    );
    let params_vec = build_expand_params(&entity_ids, project, memory_type, branch, limit * 3);
    let refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|value| value.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    let mut expanded_ids = Vec::new();
    for row in rows {
        let id = row?;
        if !seed_set.contains(&id) && !exclude_set.contains(&id) {
            expanded_ids.push(id);
            if expanded_ids.len() >= limit as usize {
                break;
            }
        }
    }
    Ok(expanded_ids)
}
