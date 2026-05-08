use anyhow::Result;
use rusqlite::{params, Connection};

pub fn enqueue_pending(
    conn: &Connection,
    host: &str,
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
         (host, session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch, updated_at_epoch, \
          status, attempt_count, next_retry_epoch, last_error, lease_owner, lease_expires_epoch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 'pending', 0, NULL, NULL, NULL, NULL)",
        params![
            host,
            session_id,
            project,
            tool_name,
            tool_input,
            tool_response,
            cwd,
            epoch
        ],
    )?;
    Ok(conn.last_insert_rowid())
}
