use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::db_job::JobType;

pub fn enqueue_job(
    conn: &Connection,
    job_type: JobType,
    project: &str,
    session_id: Option<&str>,
    payload_json: &str,
    priority: i64,
) -> Result<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM jobs
             WHERE job_type = ?1
               AND project = ?2
               AND COALESCE(session_id, '') = COALESCE(?3, '')
               AND state IN ('pending', 'processing')
             ORDER BY id DESC
             LIMIT 1",
            params![job_type.as_str(), project, session_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }

    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO jobs
         (job_type, project, session_id, payload_json, state, priority,
          attempt_count, max_attempts, lease_owner, lease_expires_epoch,
          next_retry_epoch, last_error, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, 'pending', ?5, 0, 6, NULL, NULL, ?6, NULL, ?6, ?6)",
        params![
            job_type.as_str(),
            project,
            session_id,
            payload_json,
            priority,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}
