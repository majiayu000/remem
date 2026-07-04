use anyhow::Result;
use rusqlite::{params, Connection};

use super::types::FailedPendingRow;

pub fn list_failed(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<FailedPendingRow>> {
    let limit = limit.max(1);
    let mut rows_out = Vec::new();

    if let Some(project) = project {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, project, tool_name, attempt_count, updated_at_epoch, last_error
             FROM pending_observations
             WHERE status = 'failed' AND project = ?1
             ORDER BY updated_at_epoch DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![project, limit], FailedPendingRow::from_row)?;
        for row in rows {
            rows_out.push(row?);
        }
        return Ok(rows_out);
    }

    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, tool_name, attempt_count, updated_at_epoch, last_error
         FROM pending_observations
         WHERE status = 'failed'
         ORDER BY updated_at_epoch DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], FailedPendingRow::from_row)?;
    for row in rows {
        rows_out.push(row?);
    }
    Ok(rows_out)
}

pub fn count_failed_retry_candidates(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<usize> {
    let limit = limit.max(1);
    let count: i64 = if let Some(project) = project {
        conn.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT id FROM pending_observations
                 WHERE status = 'failed'
                   AND archived_at_epoch IS NULL
                   AND project = ?1
                 ORDER BY updated_at_epoch DESC
                 LIMIT ?2
             )",
            params![project, limit],
            |row| row.get(0),
        )?
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT id FROM pending_observations
                 WHERE status = 'failed'
                   AND archived_at_epoch IS NULL
                 ORDER BY updated_at_epoch DESC
                 LIMIT ?1
             )",
            params![limit],
            |row| row.get(0),
        )?
    };
    Ok(count.max(0) as usize)
}

pub fn count_failed_purge_candidates(
    conn: &Connection,
    project: Option<&str>,
    older_than_days: i64,
) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - older_than_days.max(0) * 86_400;
    let count: i64 = if let Some(project) = project {
        conn.query_row(
            "SELECT COUNT(*) FROM pending_observations
             WHERE status = 'failed' AND project = ?1 AND updated_at_epoch < ?2",
            params![project, cutoff],
            |row| row.get(0),
        )?
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM pending_observations
             WHERE status = 'failed' AND updated_at_epoch < ?1",
            params![cutoff],
            |row| row.get(0),
        )?
    };
    Ok(count.max(0) as usize)
}
