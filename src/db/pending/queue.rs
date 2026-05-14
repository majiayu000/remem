use anyhow::Result;
use rusqlite::{params, Connection};

pub(super) const MAX_PENDING_FIELD_BYTES: usize = 16 * 1024;

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
    let tool_input = bound_pending_field(tool_input);
    let tool_response = bound_pending_field(tool_response);
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
            tool_input.as_deref(),
            tool_response.as_deref(),
            cwd,
            epoch
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn bound_pending_field(value: Option<&str>) -> Option<String> {
    value.map(|value| {
        if value.len() <= MAX_PENDING_FIELD_BYTES {
            return value.to_string();
        }

        let marker = format!(
            "\n[remem truncated legacy pending field: original_bytes={}]\n",
            value.len()
        );
        let keep_bytes = MAX_PENDING_FIELD_BYTES.saturating_sub(marker.len());
        let kept = crate::db::truncate_str(value, keep_bytes);
        format!("{}{}", kept, marker)
    })
}
