use anyhow::{ensure, Result};
use rusqlite::{params, Connection};

pub(super) fn mark_dream_generated(conn: &Connection, memory_id: i64) -> Result<()> {
    let changed = conn.execute(
        "UPDATE memories
         SET source_trust_class = 'external_content'
         WHERE id = ?1",
        params![memory_id],
    )?;
    ensure!(
        changed == 1,
        "dream_generated_trust_update_failed memory_id={memory_id}"
    );
    Ok(())
}
