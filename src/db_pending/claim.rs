use anyhow::Result;
use rusqlite::{params, Connection};

use crate::db_pending::helpers::{append_ids, clamp_error, id_placeholders};
use crate::db_pending::types::PendingObservation;

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
                updated_at_epoch, status, attempt_count, next_retry_epoch, last_error
         FROM pending_observations
         WHERE session_id = ?1 AND lease_owner = ?2 AND status = 'processing'
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(
        params![session_id, lease_owner],
        PendingObservation::from_row,
    )?;
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
