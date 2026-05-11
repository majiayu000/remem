use anyhow::Result;
use rusqlite::{params, Connection};

pub fn link_entities(conn: &Connection, memory_id: i64, entities: &[String]) -> Result<()> {
    for name in entities {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        conn.execute(
            "INSERT INTO entities (canonical_name, entity_type, mention_count)
             VALUES (?1, NULL, 1)
             ON CONFLICT(canonical_name) DO UPDATE SET mention_count = mention_count + 1",
            params![name],
        )?;
        let entity_id: i64 = conn.query_row(
            "SELECT id FROM entities WHERE canonical_name = ?1 COLLATE NOCASE",
            params![name],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO memory_entities (memory_id, entity_id) VALUES (?1, ?2)",
            params![memory_id, entity_id],
        )?;
    }
    Ok(())
}
