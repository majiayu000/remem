use anyhow::Result;
use rusqlite::Connection;

use crate::db::{self, CaptureEventInput, ExtractionTaskKind};

use super::{run_migrations, validate_schema_invariants};

#[test]
fn v078_links_one_event_projection_and_keeps_audit_writers_nullable() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    run_migrations(&conn)?;

    assert!(validate_schema_invariants(&conn)?.is_empty());
    let index_sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master
         WHERE type = 'index' AND name = 'idx_events_captured_event'",
        [],
        |row| row.get(0),
    )?;
    assert!(index_sql.contains("UNIQUE INDEX"));
    assert!(index_sql.contains("WHERE captured_event_id IS NOT NULL"));

    let capture = db::record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id: "v078-session",
            project: "/repo",
            cwd: Some("/repo"),
            event_type: "tool_result",
            role: None,
            tool_name: Some("Edit"),
            content: "edited src/lib.rs",
            task_kind: Some(ExtractionTaskKind::ObservationExtract),
        },
    )?
    .event_row_id;
    let first = crate::memory::insert_event_for_capture(
        &conn,
        capture,
        "v078-session",
        "/repo",
        "file_edit",
        "Edited src/lib.rs",
        None,
        Some("[\"src/lib.rs\"]"),
        None,
    )?;
    let replay = crate::memory::insert_event_for_capture(
        &conn,
        capture,
        "v078-session",
        "/repo",
        "file_edit",
        "Edited src/lib.rs",
        None,
        Some("[\"src/lib.rs\"]"),
        None,
    )?;
    assert_eq!(replay, first);

    let audit = crate::memory::insert_event(
        &conn,
        "audit-session",
        "/repo",
        "memory_governance",
        "Archived memory",
        None,
        None,
        None,
    )?;
    let links: (Option<i64>, Option<i64>) = conn.query_row(
        "SELECT
             (SELECT captured_event_id FROM events WHERE id = ?1),
             (SELECT captured_event_id FROM events WHERE id = ?2)",
        [first, audit],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(links, (Some(capture), None));

    conn.execute("DELETE FROM captured_events WHERE id = ?1", [capture])?;
    let link_after_delete: Option<i64> = conn.query_row(
        "SELECT captured_event_id FROM events WHERE id = ?1",
        [first],
        |row| row.get(0),
    )?;
    assert_eq!(link_after_delete, None);
    Ok(())
}
