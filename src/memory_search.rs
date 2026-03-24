use anyhow::Result;
use rusqlite::Connection;

use crate::db;
use crate::memory::{map_memory_row_pub, Memory};

/// Push project suffix-match filter into SQL conditions.
/// "harness" matches exact "harness" OR ends with "/harness".
/// Returns the next parameter index.
pub fn push_project_suffix_filter(
    column: &str,
    project: Option<&str>,
    mut idx: usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> usize {
    if let Some(p) = project {
        conditions.push(format!("({column} = ?{idx} OR {column} LIKE ?{})", idx + 1));
        params.push(Box::new(p.to_string()));
        params.push(Box::new(format!("%/{p}")));
        idx += 2;
    }
    idx
}

/// FTS5 trigram search on memories.
pub fn search_memories_fts(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Memory>> {
    let mut conditions = vec!["memories_fts MATCH ?1".to_string()];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(query.to_string()));

    let mut idx = 2;
    conditions.push("m.status = 'active'".to_string());

    idx = push_project_suffix_filter("m.project", project, idx, &mut conditions, &mut param_values);
    if let Some(t) = memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        param_values.push(Box::new(t.to_string()));
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

/// LIKE fallback for short tokens.
pub fn search_memories_like(
    conn: &Connection,
    tokens: &[&str],
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Memory>> {
    if tokens.is_empty() {
        return Ok(vec![]);
    }

    let mut conditions = vec!["m.status = 'active'".to_string()];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;

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

    idx = push_project_suffix_filter("m.project", project, idx, &mut conditions, &mut param_values);
    if let Some(t) = memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        param_values.push(Box::new(t.to_string()));
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
