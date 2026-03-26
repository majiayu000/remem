use anyhow::Result;
use rusqlite::{params, Connection};

#[derive(Debug, Clone)]
pub struct FailedPendingRow {
    pub id: i64,
    pub session_id: String,
    pub project: String,
    pub tool_name: String,
    pub attempt_count: i64,
    pub updated_at_epoch: i64,
    pub last_error: Option<String>,
}

pub fn list_failed(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<FailedPendingRow>> {
    let mut rows_out = Vec::new();
    let limit = limit.max(1);
    if let Some(project) = project {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, project, tool_name, attempt_count, updated_at_epoch, last_error
             FROM pending_observations
             WHERE status = 'failed' AND project = ?1
             ORDER BY updated_at_epoch DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![project, limit], |r| {
            Ok(FailedPendingRow {
                id: r.get(0)?,
                session_id: r.get(1)?,
                project: r.get(2)?,
                tool_name: r.get(3)?,
                attempt_count: r.get(4)?,
                updated_at_epoch: r.get(5)?,
                last_error: r.get(6)?,
            })
        })?;
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
    let rows = stmt.query_map(params![limit], |r| {
        Ok(FailedPendingRow {
            id: r.get(0)?,
            session_id: r.get(1)?,
            project: r.get(2)?,
            tool_name: r.get(3)?,
            attempt_count: r.get(4)?,
            updated_at_epoch: r.get(5)?,
            last_error: r.get(6)?,
        })
    })?;
    for row in rows {
        rows_out.push(row?);
    }
    Ok(rows_out)
}

pub fn retry_failed(conn: &Connection, project: Option<&str>, limit: i64) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let limit = limit.max(1);
    let sql = if project.is_some() {
        "UPDATE pending_observations
         SET status = 'pending',
             next_retry_epoch = NULL,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             last_error = NULL,
             updated_at_epoch = ?1
         WHERE id IN (
             SELECT id FROM pending_observations
             WHERE status = 'failed' AND project = ?2
             ORDER BY updated_at_epoch DESC
             LIMIT ?3
         )"
    } else {
        "UPDATE pending_observations
         SET status = 'pending',
             next_retry_epoch = NULL,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             last_error = NULL,
             updated_at_epoch = ?1
         WHERE id IN (
             SELECT id FROM pending_observations
             WHERE status = 'failed'
             ORDER BY updated_at_epoch DESC
             LIMIT ?2
         )"
    };

    let n = if let Some(project) = project {
        conn.execute(sql, params![now, project, limit])?
    } else {
        conn.execute(sql, params![now, limit])?
    };
    Ok(n)
}

pub fn purge_failed(
    conn: &Connection,
    project: Option<&str>,
    older_than_days: i64,
) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - older_than_days.max(0) * 86_400;
    let n = if let Some(project) = project {
        conn.execute(
            "DELETE FROM pending_observations
             WHERE status = 'failed' AND project = ?1 AND updated_at_epoch < ?2",
            params![project, cutoff],
        )?
    } else {
        conn.execute(
            "DELETE FROM pending_observations
             WHERE status = 'failed' AND updated_at_epoch < ?1",
            params![cutoff],
        )?
    };
    Ok(n)
}
