use anyhow::Result;
use rusqlite::{params, Connection};

pub fn cleanup_old_events(conn: &Connection, days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (days * 86400);
    Ok(conn.execute(
        "DELETE FROM events WHERE created_at_epoch < ?1",
        params![cutoff],
    )?)
}

pub fn archive_stale_memories(conn: &Connection, days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (days * 86400);
    Ok(conn.execute(
        "UPDATE memories SET status = 'archived' \
         WHERE status = 'active' AND updated_at_epoch < ?1",
        params![cutoff],
    )?)
}
