use anyhow::Result;
use rusqlite::{params, Connection};

use crate::memory::types::{map_event_row, Event};

pub fn get_session_events(conn: &Connection, session_id: &str) -> Result<Vec<Event>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, event_type, summary, detail, files, exit_code, \
         created_at_epoch FROM events WHERE session_id = ?1 ORDER BY created_at_epoch ASC",
    )?;
    let rows = stmt.query_map(params![session_id], map_event_row)?;
    crate::db_query::collect_rows(rows)
}

pub fn get_recent_events(conn: &Connection, project: &str, limit: i64) -> Result<Vec<Event>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, event_type, summary, detail, files, exit_code, \
         created_at_epoch FROM events WHERE project = ?1 ORDER BY created_at_epoch DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project, limit], map_event_row)?;
    crate::db_query::collect_rows(rows)
}

pub fn count_session_memories(conn: &Connection, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?)
}

pub fn get_session_files_modified(conn: &Connection, session_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT files FROM events \
         WHERE session_id = ?1 AND event_type IN ('file_edit', 'file_create') AND files IS NOT NULL",
    )?;
    let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;

    let mut files = Vec::new();
    for row in rows {
        let files_json = row?;
        if let Ok(entries) = serde_json::from_str::<Vec<String>>(&files_json) {
            for entry in entries {
                if !files.contains(&entry) {
                    files.push(entry);
                }
            }
        }
    }
    Ok(files)
}

pub fn count_session_events(conn: &Connection, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?)
}
