use anyhow::Result;
use rusqlite::{params, Connection};

fn event_retention_class(event_type: &str) -> &'static str {
    match event_type {
        "file_edit"
        | "file_create"
        | "bash"
        | "search"
        | "agent"
        | "tool_result"
        | "cursor_tool_failure" => "ephemeral",
        _ => "audit",
    }
}

pub fn insert_event(
    conn: &Connection,
    session_id: &str,
    project: &str,
    event_type: &str,
    summary: &str,
    detail: Option<&str>,
    files: Option<&str>,
    exit_code: Option<i32>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO events \
         (session_id, project, event_type, summary, detail, files, exit_code,
          created_at_epoch, retention_class) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            session_id,
            project,
            event_type,
            summary,
            detail,
            files,
            exit_code,
            now,
            event_retention_class(event_type)
        ],
    )?;
    Ok(conn.last_insert_rowid())
}
