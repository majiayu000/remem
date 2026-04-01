use anyhow::Result;
use rusqlite::{params, Connection};

pub fn get_stale_pending_sessions(
    conn: &Connection,
    project: &str,
    age_secs: i64,
) -> Result<Vec<String>> {
    let cutoff = chrono::Utc::now().timestamp() - age_secs;
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn.prepare(
        "SELECT DISTINCT session_id FROM pending_observations \
         WHERE project = ?1
           AND status = 'pending'
           AND created_at_epoch < ?2
           AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?3)
           AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?3)",
    )?;
    let rows = stmt.query_map(params![project, cutoff, now], |row| row.get(0))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn count_pending(conn: &Connection, session_id: &str) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations
         WHERE session_id = ?1
           AND status = 'pending'
           AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?2)
           AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?2)",
        params![session_id, now],
        |row| row.get(0),
    )?;
    Ok(count)
}
