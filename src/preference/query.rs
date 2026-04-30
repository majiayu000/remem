use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashSet;

use crate::memory::{self, Memory};

pub fn query_global_preferences(conn: &Connection, limit: usize) -> Result<Vec<Memory>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let mut results = query_explicit_global_preferences(conn, limit)?;
    if results.len() >= limit {
        return Ok(results);
    }

    let mut seen_keys: HashSet<String> = results.iter().map(global_preference_key).collect();
    let derived = query_derived_global_preferences(conn, limit - results.len())?;
    for memory in derived {
        if seen_keys.insert(global_preference_key(&memory)) {
            results.push(memory);
        }
        if results.len() >= limit {
            break;
        }
    }
    Ok(results)
}

fn query_explicit_global_preferences(conn: &Connection, limit: usize) -> Result<Vec<Memory>> {
    let sql = format!(
        "SELECT {} FROM memories \
         WHERE memory_type = 'preference' AND status = 'active' \
         AND scope = 'global' \
         GROUP BY COALESCE(topic_key, id) \
         ORDER BY MAX(updated_at_epoch) DESC LIMIT ?1",
        memory::MEMORY_COLS,
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![limit as i64], memory::map_memory_row_pub)?;
    crate::db_query::collect_rows(rows)
}

fn query_derived_global_preferences(conn: &Connection, limit: usize) -> Result<Vec<Memory>> {
    let sql = format!(
        "SELECT {} FROM memories \
         WHERE memory_type = 'preference' AND status = 'active' \
         AND topic_key IS NOT NULL AND topic_key IN ( \
             SELECT topic_key FROM memories \
             WHERE memory_type = 'preference' AND status = 'active' AND topic_key IS NOT NULL \
             GROUP BY topic_key HAVING COUNT(DISTINCT project) >= 3 \
         ) \
         GROUP BY COALESCE(topic_key, id) \
         ORDER BY MAX(updated_at_epoch) DESC LIMIT ?1",
        memory::MEMORY_COLS,
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![limit as i64], memory::map_memory_row_pub)?;
    crate::db_query::collect_rows(rows)
}

fn global_preference_key(memory: &Memory) -> String {
    memory
        .topic_key
        .clone()
        .unwrap_or_else(|| format!("id:{}", memory.id))
}
