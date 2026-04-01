use anyhow::Result;
use rusqlite::{params, Connection};

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
         (session_id, project, event_type, summary, detail, files, exit_code, created_at_epoch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![session_id, project, event_type, summary, detail, files, exit_code, now],
    )?;
    Ok(conn.last_insert_rowid())
}
