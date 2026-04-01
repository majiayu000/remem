use anyhow::Result;
use rusqlite::{params, Connection};

use super::types::{map_event_row, Event};

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

pub fn cleanup_old_events(conn: &Connection, days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (days * 86400);
    Ok(conn.execute(
        "DELETE FROM events WHERE created_at_epoch < ?1",
        params![cutoff],
    )?)
}

pub fn archive_stale_memories(conn: &Connection, days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (days * 86400);
    Ok(conn.execute(
        "UPDATE memories SET status = 'archived' \
         WHERE status = 'active' AND updated_at_epoch < ?1",
        params![cutoff],
    )?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::tests_helper::setup_memory_schema;
    use rusqlite::Connection;

    #[test]
    fn test_event_insert_and_query() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        insert_event(
            &conn,
            "session-1",
            "proj",
            "file_edit",
            "Edit src/db.rs",
            None,
            Some(r#"["src/db.rs"]"#),
            None,
        )
        .unwrap();
        insert_event(
            &conn,
            "session-1",
            "proj",
            "bash",
            "Run `cargo test` (exit 0)",
            None,
            None,
            Some(0),
        )
        .unwrap();

        let events = get_session_events(&conn, "session-1").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "file_edit");
        assert_eq!(events[1].exit_code, Some(0));
    }

    #[test]
    fn test_cleanup_old_events() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let now = chrono::Utc::now().timestamp();
        let old = now - (31 * 86400);
        conn.execute(
            "INSERT INTO events (session_id, project, event_type, summary, created_at_epoch)
             VALUES ('s1', 'proj', 'file_edit', 'old edit', ?1)",
            params![old],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (session_id, project, event_type, summary, created_at_epoch)
             VALUES ('s2', 'proj', 'file_edit', 'new edit', ?1)",
            params![now],
        )
        .unwrap();

        assert_eq!(cleanup_old_events(&conn, 30).unwrap(), 1);
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_archive_stale_memories() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let now = chrono::Utc::now().timestamp();
        let old = now - (181 * 86400);
        conn.execute(
            "INSERT INTO memories (session_id, project, title, content, memory_type, \
             created_at_epoch, updated_at_epoch, status)
             VALUES ('s1', 'proj', 'old', 'old content', 'decision', ?1, ?1, 'active')",
            params![old],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (session_id, project, title, content, memory_type, \
             created_at_epoch, updated_at_epoch, status)
             VALUES ('s2', 'proj', 'new', 'new content', 'decision', ?1, ?1, 'active')",
            params![now],
        )
        .unwrap();

        assert_eq!(archive_stale_memories(&conn, 180).unwrap(), 1);
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE status = 'active'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 1);
    }
}
