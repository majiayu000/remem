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
         AND {} \
         AND ((owner_scope = 'repo' AND owner_key = ?1) \
              OR (owner_scope = 'repo' AND target_project = ?1) \
              OR (owner_scope IS NULL AND project = ?1 AND (scope IS NULL OR scope = 'project'))) \
         GROUP BY COALESCE(owner_scope, 'legacy_project'),
                  COALESCE(owner_key, project),
                  COALESCE(topic_key, id) \
         ORDER BY MAX(updated_at_epoch) DESC LIMIT ?2",
        memory::MEMORY_COLS,
        memory::memory_current_filter_sql("status", "expires_at_epoch", false),
        memory::memory_state_key_current_filter_sql("memories"),
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
         AND {} \
         AND ((owner_scope = 'user' AND owner_key = 'user:default') \
              OR (owner_scope IS NULL AND scope = 'global')) \
         GROUP BY COALESCE(owner_scope, 'legacy_global'),
                  COALESCE(owner_key, scope, 'global'),
                  COALESCE(topic_key, id) \
         ORDER BY MAX(updated_at_epoch) DESC LIMIT ?1",
        memory::MEMORY_COLS,
        memory::memory_current_filter_sql("status", "expires_at_epoch", false),
        memory::memory_state_key_current_filter_sql("memories"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![limit as i64], memory::map_memory_row_pub)?;
    crate::db::query::collect_rows(rows)
}
