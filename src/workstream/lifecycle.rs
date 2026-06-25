use anyhow::Result;
use rusqlite::{params, Connection};

pub const DEFAULT_AUTO_PAUSE_DAYS: i64 = 14;
pub const DEFAULT_AUTO_ABANDON_DAYS: i64 = 30;

pub fn auto_pause_inactive(conn: &Connection, project: &str, days: i64) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let cutoff = cutoff_epoch(now, days);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'paused', updated_at_epoch = ?3
         WHERE status = 'active'
           AND updated_at_epoch < ?2
           AND merged_into_workstream_id IS NULL
           AND ((owner_scope = 'repo' AND owner_key = ?1)
                OR (owner_scope = 'repo' AND target_project = ?1)
                OR (owner_scope = 'workstream' AND target_project = ?1)
                OR (owner_scope IS NULL AND project = ?1))",
        params![project, cutoff, now],
    )?;
    Ok(count)
}

pub fn auto_pause_all_inactive(conn: &Connection, days: i64) -> Result<usize> {
    auto_pause_all_inactive_at(conn, chrono::Utc::now().timestamp(), days)
}

pub fn auto_pause_all_inactive_at(conn: &Connection, now_epoch: i64, days: i64) -> Result<usize> {
    let cutoff = cutoff_epoch(now_epoch, days);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'paused', updated_at_epoch = ?2
         WHERE status = 'active'
           AND updated_at_epoch < ?1
           AND merged_into_workstream_id IS NULL",
        params![cutoff, now_epoch],
    )?;
    Ok(count)
}

pub fn count_auto_pause_all_inactive_at(
    conn: &Connection,
    now_epoch: i64,
    days: i64,
) -> Result<usize> {
    let cutoff = cutoff_epoch(now_epoch, days);
    count_rows(
        conn,
        "SELECT COUNT(*) FROM workstreams
          WHERE status = 'active'
            AND updated_at_epoch < ?1
            AND merged_into_workstream_id IS NULL",
        &[&cutoff],
    )
}

pub fn auto_abandon_inactive(conn: &Connection, project: &str, days: i64) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let cutoff = cutoff_epoch(now, days);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'abandoned', updated_at_epoch = ?1
         WHERE status = 'paused'
           AND updated_at_epoch < ?3
           AND merged_into_workstream_id IS NULL
           AND ((owner_scope = 'repo' AND owner_key = ?2)
                OR (owner_scope = 'repo' AND target_project = ?2)
                OR (owner_scope = 'workstream' AND target_project = ?2)
                OR (owner_scope IS NULL AND project = ?2))",
        params![now, project, cutoff],
    )?;
    Ok(count)
}

pub fn auto_abandon_all_inactive(conn: &Connection, days: i64) -> Result<usize> {
    auto_abandon_all_inactive_at(conn, chrono::Utc::now().timestamp(), days)
}

pub fn auto_abandon_all_inactive_at(conn: &Connection, now_epoch: i64, days: i64) -> Result<usize> {
    let cutoff = cutoff_epoch(now_epoch, days);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'abandoned', updated_at_epoch = ?1
         WHERE status = 'paused'
           AND updated_at_epoch < ?2
           AND merged_into_workstream_id IS NULL",
        params![now_epoch, cutoff],
    )?;
    Ok(count)
}

pub fn count_auto_abandon_all_inactive_at(
    conn: &Connection,
    now_epoch: i64,
    days: i64,
) -> Result<usize> {
    let cutoff = cutoff_epoch(now_epoch, days);
    count_rows(
        conn,
        "SELECT COUNT(*) FROM workstreams
          WHERE status = 'paused'
            AND updated_at_epoch < ?1
            AND merged_into_workstream_id IS NULL",
        &[&cutoff],
    )
}

fn cutoff_epoch(now_epoch: i64, days: i64) -> i64 {
    now_epoch.saturating_sub(days.saturating_mul(86_400))
}

fn count_rows(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::types::ToSql],
) -> Result<usize> {
    let count: i64 = conn.query_row(sql, params, |row| row.get(0))?;
    Ok(count as usize)
}
