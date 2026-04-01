use anyhow::Result;
use rusqlite::{params, Connection};

use crate::memory::{self, Memory};

pub fn query_global_preferences(conn: &Connection, limit: usize) -> Result<Vec<Memory>> {
    let sql = format!(
        "SELECT {} FROM memories \
         WHERE memory_type = 'preference' AND status = 'active' \
         AND (scope = 'global' OR (topic_key IS NOT NULL AND topic_key IN ( \
             SELECT topic_key FROM memories \
             WHERE memory_type = 'preference' AND status = 'active' AND topic_key IS NOT NULL \
             GROUP BY topic_key HAVING COUNT(DISTINCT project) >= 3 \
         ))) \
         GROUP BY COALESCE(topic_key, id) \
         ORDER BY MAX(updated_at_epoch) DESC LIMIT ?1",
        memory::MEMORY_COLS,
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![limit as i64], memory::map_memory_row_pub)?;
    crate::db_query::collect_rows(rows)
}
