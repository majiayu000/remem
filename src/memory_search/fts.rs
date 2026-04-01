use anyhow::Result;
use rusqlite::Connection;

use crate::db;
use crate::memory::{map_memory_row_pub, Memory};
use crate::memory_search::filters::{push_branch_filter, push_project_filter};

/// FTS5 trigram search on memories.
pub fn search_memories_fts(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Memory>> {
    search_memories_fts_filtered(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        false,
        None,
    )
}

pub fn search_memories_fts_filtered(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_inactive: bool,
    branch: Option<&str>,
) -> Result<Vec<Memory>> {
    let mut conditions = vec!["memories_fts MATCH ?1".to_string()];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(query.to_string())];

    let mut idx = 2;
    if !include_inactive {
        conditions.push("m.status = 'active'".to_string());
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
         JOIN memories_fts ON memories_fts.rowid = m.id \
         WHERE {} \
         ORDER BY (bm25(memories_fts, 10.0, 1.0) * CASE WHEN m.memory_type IN ('decision','bugfix') THEN 1.5 ELSE 1.0 END) \
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
