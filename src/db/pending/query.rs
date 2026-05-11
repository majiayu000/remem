use anyhow::Result;
use rusqlite::{params, Connection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingIdentity {
    pub host: String,
    pub project: String,
    pub session_id: String,
    pub ready_count: i64,
    pub oldest_epoch: i64,
}

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

pub fn get_stale_pending_identities(
    conn: &Connection,
    age_secs: i64,
    limit: i64,
) -> Result<Vec<PendingIdentity>> {
    let cutoff = chrono::Utc::now().timestamp() - age_secs;
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn.prepare(
        "SELECT host, project, session_id, COUNT(*) AS ready_count, MIN(created_at_epoch) AS oldest_epoch
         FROM pending_observations
         WHERE status = 'pending'
           AND created_at_epoch < ?1
           AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?2)
           AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?2)
         GROUP BY host, project, session_id
         ORDER BY oldest_epoch ASC, ready_count DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![cutoff, now, limit.max(1)], |row| {
        Ok(PendingIdentity {
            host: row.get(0)?,
            project: row.get(1)?,
            session_id: row.get(2)?,
            ready_count: row.get(3)?,
            oldest_epoch: row.get(4)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn count_pending(conn: &Connection, session_id: &str) -> Result<i64> {
    count_pending_for_identity(conn, "unknown", "", session_id)
}

pub fn count_pending_for_identity(
    conn: &Connection,
    host: &str,
    project: &str,
    session_id: &str,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations
         WHERE (host = ?1 OR host = 'unknown')
           AND (?2 = '' OR project = ?2)
           AND session_id = ?3
           AND status = 'pending'
           AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?4)
           AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?4)",
        params![host, project, session_id, now],
        |row| row.get(0),
    )?;
    Ok(count)
}
