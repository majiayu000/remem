use anyhow::Result;
use rusqlite::{params, Connection};

use crate::memory::{self, Memory};

pub fn query_project_preferences(
    conn: &Connection,
    project: &str,
    limit: usize,
) -> Result<Vec<Memory>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let sql = format!(
        "SELECT {} FROM memories \
         WHERE memory_type = 'preference' AND {} \
         AND ((owner_scope = 'repo' AND owner_key = ?1) \
              OR (owner_scope = 'repo' AND target_project = ?1) \
              OR (owner_scope IS NULL AND project = ?1 AND (scope IS NULL OR scope = 'project'))) \
         GROUP BY COALESCE(topic_key, id) \
         ORDER BY MAX(updated_at_epoch) DESC LIMIT ?2",
        memory::MEMORY_COLS,
        memory::memory_current_filter_sql("status", "expires_at_epoch", false),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![project, limit as i64], memory::map_memory_row_pub)?;
    crate::db::query::collect_rows(rows)
}

pub fn query_global_preferences(conn: &Connection, limit: usize) -> Result<Vec<Memory>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let sql = format!(
        "SELECT {} FROM memories \
         WHERE memory_type = 'preference' AND {} \
         AND ((owner_scope = 'user' AND owner_key = 'user:default') \
              OR (owner_scope IS NULL AND scope = 'global')) \
         GROUP BY COALESCE(topic_key, id) \
         ORDER BY MAX(updated_at_epoch) DESC LIMIT ?1",
        memory::MEMORY_COLS,
        memory::memory_current_filter_sql("status", "expires_at_epoch", false),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![limit as i64], memory::map_memory_row_pub)?;
    crate::db::query::collect_rows(rows)
}
