use anyhow::Result;
use rusqlite::{params, Connection};

pub fn auto_pause_inactive(conn: &Connection, project: &str, days: i64) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - (days * 86400);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'paused', updated_at_epoch = ?1
         WHERE project = ?2 AND status = 'active' AND updated_at_epoch < ?3",
        params![now, project, cutoff],
    )?;
    Ok(count)
}

pub fn auto_abandon_inactive(conn: &Connection, project: &str, days: i64) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - (days * 86400);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'abandoned', updated_at_epoch = ?1
         WHERE project = ?2 AND status = 'paused' AND updated_at_epoch < ?3",
        params![now, project, cutoff],
    )?;
    Ok(count)
}
