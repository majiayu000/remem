use anyhow::Result;
use rusqlite::{params, Connection};

use crate::pending_admin::types::FailedPendingRow;

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
