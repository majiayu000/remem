use anyhow::Result;
use rusqlite::Connection;

use crate::db;
use crate::memory::{map_memory_row_pub, Memory};
use crate::memory_search::filters::{push_branch_filter, push_project_filter};

/// LIKE fallback for short tokens.
pub fn search_memories_like(
    conn: &Connection,
    tokens: &[&str],
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Memory>> {
    search_memories_like_filtered(
        conn,
        tokens,
        project,
        memory_type,
        limit,
        offset,
        false,
        None,
    )
}

pub fn search_memories_like_filtered(
    conn: &Connection,
    tokens: &[&str],
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_inactive: bool,
    branch: Option<&str>,
) -> Result<Vec<Memory>> {
    if tokens.is_empty() {
        return Ok(vec![]);
    }

    let mut conditions = Vec::new();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;

    if !include_inactive {
        conditions.push("m.status = 'active'".to_string());
    }

    for token in tokens {
        let like_pattern = format!("%{token}%");
        let cols = ["m.title", "m.content"];
        let token_clauses: Vec<String> = cols
            .iter()
            .map(|col| format!("{col} LIKE ?{idx}"))
            .collect();
        param_values.push(Box::new(like_pattern));
        conditions.push(format!("({})", token_clauses.join(" OR ")));
        idx += 1;
    }

    idx = push_project_filter(
        "m.project",
        project,
        idx,
        &mut conditions,
        &mut param_values,
    );
    idx = push_branch_filter("m.branch", branch, idx, &mut conditions, &mut param_values);
    if let Some(memory_type) = memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        param_values.push(Box::new(memory_type.to_string()));
        idx += 1;
    }

    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

    let sql = format!(
        "SELECT m.id, m.session_id, m.project, m.topic_key, m.title, m.content, \
         m.memory_type, m.files, m.created_at_epoch, m.updated_at_epoch, m.status, m.branch, m.scope \
         FROM memories m \
         WHERE {} \
         ORDER BY m.updated_at_epoch DESC \
         LIMIT ?{} OFFSET ?{}",
        conditions.join(" AND "),
        idx,
        idx + 1
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row_pub)?;
    crate::db_query::collect_rows(rows)
}
