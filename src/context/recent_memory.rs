use anyhow::Result;
use rusqlite::Connection;

use crate::memory;

use super::filters::{push_excluded_type_filter, push_owner_included_filter};
use super::query::{map_context_memory_row, ContextMemoryRow, MEMORY_OWNER_COLS};

pub(super) fn query_owner_included_memory_rows(
    conn: &Connection,
    project: &str,
    query: Option<&str>,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    offset: i64,
    limit: i64,
) -> Result<Vec<ContextMemoryRow>> {
    if offset < 0 || limit <= 0 || query.is_some_and(|value| value.trim().is_empty()) {
        return Ok(vec![]);
    }

    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    push_owner_included_filter(conn, project, &mut idx, &mut conditions, &mut params)?;
    conditions.push(crate::memory::memory_current_filter_sql(
        "status",
        "expires_at_epoch",
        false,
    ));
    conditions.push(crate::memory::memory_state_key_current_filter_sql(
        "memories",
    ));
    conditions.push(crate::memory::suppression::memory_policy_filter_sql(
        "memories",
    ));
    if let Some(branch) = current_branch.filter(|branch| !branch.trim().is_empty()) {
        conditions.push(format!("(branch = ?{idx} OR branch IS NULL)"));
        params.push(Box::new(branch.to_string()));
        idx += 1;
    }

    if let Some(query) = query {
        let like_pattern = format!("%{query}%");
        conditions.push(format!("(title LIKE ?{idx} OR content LIKE ?{idx})"));
        params.push(Box::new(like_pattern));
        idx += 1;
    }

    push_excluded_type_filter(excluded_types, &mut idx, &mut conditions, &mut params);
    params.push(Box::new(limit));
    params.push(Box::new(offset));
    let sql = format!(
        "SELECT {}, {} FROM memories \
         WHERE {} \
         ORDER BY updated_at_epoch DESC, id ASC LIMIT ?{} OFFSET ?{}",
        memory::MEMORY_COLS,
        MEMORY_OWNER_COLS,
        conditions.join(" AND "),
        idx,
        idx + 1,
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), map_context_memory_row)?;
    crate::db::query::collect_rows(rows)
}
