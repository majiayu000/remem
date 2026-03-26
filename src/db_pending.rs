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
    pub updated_at_epoch: i64,
    pub status: String,
    pub attempt_count: i64,
    pub next_retry_epoch: Option<i64>,
    pub last_error: Option<String>,
}

fn clamp_error(error: &str) -> String {
    crate::db::truncate_str(error, 1000).to_string()
}

fn id_placeholders(ids: &[i64], start_idx: usize) -> String {
    (start_idx..start_idx + ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn append_ids(params: &mut Vec<Box<dyn rusqlite::types::ToSql>>, ids: &[i64]) {
    for id in ids {
        params.push(Box::new(*id));
    }
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
         (session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch, updated_at_epoch, \
          status, attempt_count, next_retry_epoch, last_error, lease_owner, lease_expires_epoch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 'pending', 0, NULL, NULL, NULL, NULL)",
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
         SET lease_owner = ?1,
             lease_expires_epoch = ?2,
             status = 'processing',
             attempt_count = COALESCE(attempt_count, 0) + 1,
             updated_at_epoch = ?4
         WHERE id IN (
             SELECT id FROM pending_observations
             WHERE session_id = ?3
               AND status = 'pending'
               AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?4)
               AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?4)
             ORDER BY id ASC
             LIMIT ?5
         )
           AND status = 'pending'
           AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?4)
           AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?4)",
        params![lease_owner, lease_expires, session_id, now, limit as i64],
    )?;

    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch, \
                updated_at_epoch, status, attempt_count, next_retry_epoch, last_error \
         FROM pending_observations
         WHERE session_id = ?1 AND lease_owner = ?2 AND status = 'processing'
         ORDER BY id ASC",
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
            updated_at_epoch: row.get(8)?,
            status: row.get(9)?,
            attempt_count: row.get(10)?,
            next_retry_epoch: row.get(11)?,
            last_error: row.get(12)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn release_pending_claims(conn: &Connection, lease_owner: &str) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let count = conn.execute(
        "UPDATE pending_observations
         SET lease_owner = NULL,
             lease_expires_epoch = NULL,
             status = 'pending',
             next_retry_epoch = NULL,
             updated_at_epoch = ?2
         WHERE lease_owner = ?1 AND status = 'processing'",
        params![lease_owner, now],
    )?;
    Ok(count)
}

pub fn retry_pending_claimed(
    conn: &Connection,
    lease_owner: &str,
    ids: &[i64],
    error: &str,
    retry_after_secs: i64,
) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let now = chrono::Utc::now().timestamp();
    let next_retry = now + retry_after_secs.max(0);
    let sql = format!(
        "UPDATE pending_observations
         SET lease_owner = NULL,
             lease_expires_epoch = NULL,
             status = 'pending',
             next_retry_epoch = ?3,
             last_error = ?2,
             updated_at_epoch = ?4
         WHERE lease_owner = ?1
           AND status = 'processing'
           AND id IN ({})",
        id_placeholders(ids, 5)
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(lease_owner.to_string()),
        Box::new(clamp_error(error)),
        Box::new(next_retry),
        Box::new(now),
    ];
    append_ids(&mut param_values, ids);
    let refs = crate::db::to_sql_refs(&param_values);
    Ok(stmt.execute(refs.as_slice())?)
}

pub fn fail_pending_claimed(
    conn: &Connection,
    lease_owner: &str,
    ids: &[i64],
    error: &str,
) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let now = chrono::Utc::now().timestamp();
    let sql = format!(
        "UPDATE pending_observations
         SET lease_owner = NULL,
             lease_expires_epoch = NULL,
             status = 'failed',
             next_retry_epoch = NULL,
             last_error = ?2,
             updated_at_epoch = ?3
         WHERE lease_owner = ?1
           AND status = 'processing'
           AND id IN ({})",
        id_placeholders(ids, 4)
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(lease_owner.to_string()),
        Box::new(clamp_error(error)),
        Box::new(now),
    ];
    append_ids(&mut param_values, ids);
    let refs = crate::db::to_sql_refs(&param_values);
    Ok(stmt.execute(refs.as_slice())?)
}

pub fn delete_pending_claimed(conn: &Connection, lease_owner: &str, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let sql = format!(
        "DELETE FROM pending_observations
         WHERE lease_owner = ?1
           AND status = 'processing'
           AND id IN ({})",
        id_placeholders(ids, 2)
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(lease_owner.to_string())];
    append_ids(&mut param_values, ids);
    let refs = crate::db::to_sql_refs(&param_values);
    Ok(stmt.execute(refs.as_slice())?)
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
