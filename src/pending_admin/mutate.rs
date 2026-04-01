use anyhow::Result;
use rusqlite::{params, Connection};

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

    let changed = if let Some(project) = project {
        conn.execute(sql, params![now, project, limit])?
    } else {
        conn.execute(sql, params![now, limit])?
    };
    Ok(changed)
}

pub fn purge_failed(
    conn: &Connection,
    project: Option<&str>,
    older_than_days: i64,
) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - older_than_days.max(0) * 86_400;
    let changed = if let Some(project) = project {
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
    Ok(changed)
}
