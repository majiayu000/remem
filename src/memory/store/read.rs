use anyhow::Result;
use rusqlite::Connection;

use crate::memory::types::{map_memory_row, Memory, MEMORY_COLS};
use crate::memory_search::push_project_filter_required;

pub fn get_recent_memories(conn: &Connection, project: &str, limit: i64) -> Result<Vec<Memory>> {
    list_memories(conn, project, None, limit, 0, false, None)
}

pub fn get_memories_by_type(
    conn: &Connection,
    project: &str,
    memory_type: &str,
    limit: i64,
) -> Result<Vec<Memory>> {
    list_memories(conn, project, Some(memory_type), limit, 0, false, None)
}

pub fn list_memories(
    conn: &Connection,
    project: &str,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_inactive: bool,
    branch: Option<&str>,
) -> Result<Vec<Memory>> {
    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    idx = push_project_filter_required("project", project, idx, &mut conditions, &mut params);

    if !include_inactive {
        conditions.push("status = 'active'".to_string());
    }
    if let Some(memory_type) = memory_type {
        conditions.push(format!("memory_type = ?{idx}"));
        params.push(Box::new(memory_type.to_string()));
        idx += 1;
    }
    if let Some(branch) = branch {
        conditions.push(format!("(branch = ?{idx} OR branch IS NULL)"));
        params.push(Box::new(branch.to_string()));
        idx += 1;
    }

    params.push(Box::new(limit));
    params.push(Box::new(offset.max(0)));
    let sql = format!(
        "SELECT {} FROM memories \
         WHERE {} \
         ORDER BY updated_at_epoch DESC LIMIT ?{} OFFSET ?{}",
        MEMORY_COLS,
        conditions.join(" AND "),
        idx,
        idx + 1,
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row)?;
    crate::db_query::collect_rows(rows)
}

pub fn get_memories_by_ids(
    conn: &Connection,
    ids: &[i64],
    project: Option<&str>,
) -> Result<Vec<Memory>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let mut conditions = vec![format!("id IN ({})", placeholders.join(", "))];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();

    if let Some(project) = project {
        let idx = ids.len() + 1;
        conditions.push(format!("(project = ?{idx} OR scope = 'global')"));
        params.push(Box::new(project.to_string()));
    }

    let sql = format!(
        "SELECT {} FROM memories WHERE {} ORDER BY updated_at_epoch DESC",
        MEMORY_COLS,
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row)?;
    crate::db_query::collect_rows(rows)
}

#[cfg(test)]
mod tests;
