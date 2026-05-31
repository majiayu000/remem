use anyhow::Result;
use rusqlite::{params, Connection};

pub const DEFAULT_AUTO_PAUSE_DAYS: i64 = 14;
pub const DEFAULT_AUTO_ABANDON_DAYS: i64 = 30;

pub fn auto_pause_inactive(conn: &Connection, project: &str, days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (days * 86400);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'paused'
         WHERE status = 'active'
           AND updated_at_epoch < ?2
           AND ((owner_scope = 'repo' AND owner_key = ?1)
                OR (owner_scope = 'repo' AND target_project = ?1)
                OR (owner_scope = 'workstream' AND target_project = ?1)
                OR (owner_scope IS NULL AND project = ?1))",
        params![project, cutoff],
    )?;
    Ok(count)
}

pub fn auto_pause_all_inactive(conn: &Connection, days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (days * 86400);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'paused'
         WHERE status = 'active' AND updated_at_epoch < ?1",
        params![cutoff],
    )?;
    Ok(count)
}

pub fn auto_abandon_inactive(conn: &Connection, project: &str, days: i64) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - (days * 86400);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'abandoned', updated_at_epoch = ?1
         WHERE status = 'paused'
           AND updated_at_epoch < ?3
           AND ((owner_scope = 'repo' AND owner_key = ?2)
                OR (owner_scope = 'repo' AND target_project = ?2)
                OR (owner_scope = 'workstream' AND target_project = ?2)
                OR (owner_scope IS NULL AND project = ?2))",
        params![now, project, cutoff],
    )?;
    Ok(count)
}

pub fn auto_abandon_all_inactive(conn: &Connection, days: i64) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - (days * 86400);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'abandoned', updated_at_epoch = ?1
         WHERE status = 'paused' AND updated_at_epoch < ?2",
        params![now, cutoff],
    )?;
    Ok(count)
}
