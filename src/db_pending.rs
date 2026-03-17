use anyhow::Result;
use rusqlite::{params, Connection};

#[derive(Debug)]
#[allow(dead_code)]
pub struct PendingObservation {
    pub id: i64,
    pub session_id: String,
    pub project: String,
    pub tool_name: String,
    pub tool_input: Option<String>,
    pub tool_response: Option<String>,
    pub cwd: Option<String>,
    pub created_at_epoch: i64,
}

pub fn enqueue_pending(
    conn: &Connection,
    session_id: &str,
    project: &str,
    tool_name: &str,
    tool_input: Option<&str>,
    tool_response: Option<&str>,
    cwd: Option<&str>,
) -> Result<i64> {
    let epoch = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO pending_observations \
         (session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch, lease_owner, lease_expires_epoch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL)",
        params![session_id, project, tool_name, tool_input, tool_response, cwd, epoch],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn claim_pending(
    conn: &Connection,
    session_id: &str,
    limit: usize,
    lease_owner: &str,
    lease_secs: i64,
) -> Result<Vec<PendingObservation>> {
    let now = chrono::Utc::now().timestamp();
    let lease_expires = now + lease_secs.max(1);
    conn.execute(
        "UPDATE pending_observations
         SET lease_owner = ?1, lease_expires_epoch = ?2
         WHERE id IN (
             SELECT id FROM pending_observations
             WHERE session_id = ?3
               AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?4)
             ORDER BY id ASC
             LIMIT ?5
         )
           AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?4)",
        params![lease_owner, lease_expires, session_id, now, limit as i64],
    )?;

    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch \
         FROM pending_observations
         WHERE session_id = ?1 AND lease_owner = ?2
         ORDER BY id ASC"
    )?;
    let rows = stmt.query_map(params![session_id, lease_owner], |row| {
        Ok(PendingObservation {
            id: row.get(0)?,
            session_id: row.get(1)?,
            project: row.get(2)?,
            tool_name: row.get(3)?,
            tool_input: row.get(4)?,
            tool_response: row.get(5)?,
            cwd: row.get(6)?,
            created_at_epoch: row.get(7)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn release_pending_claims(conn: &Connection, lease_owner: &str) -> Result<usize> {
    let count = conn.execute(
        "UPDATE pending_observations
         SET lease_owner = NULL, lease_expires_epoch = NULL
         WHERE lease_owner = ?1",
        params![lease_owner],
    )?;
    Ok(count)
}

pub fn delete_pending_claimed(conn: &Connection, lease_owner: &str, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders: Vec<String> = (2..=ids.len() + 1).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "DELETE FROM pending_observations WHERE lease_owner = ?1 AND id IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(lease_owner.to_string()));
    for id in ids {
        param_values.push(Box::new(*id));
    }
    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
    let count = stmt.execute(refs.as_slice())?;
    Ok(count)
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
         WHERE project = ?1 AND created_at_epoch < ?2 \
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
           AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?2)",
        params![session_id, now],
        |row| row.get(0),
    )?;
    Ok(count)
}
