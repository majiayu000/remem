use rusqlite::{params, Connection};

use super::{
    archive_stale_memories, cleanup_old_events, get_session_events, get_session_files_modified,
    insert_event,
};
use crate::memory::tests_helper::setup_memory_schema;

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
fn test_get_session_files_modified_dedups_entries() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    insert_event(
        &conn,
        "session-1",
        "proj",
        "file_edit",
        "Edit sources",
        None,
        Some(r#"["src/lib.rs","src/main.rs"]"#),
        None,
    )
    .unwrap();
    insert_event(
        &conn,
        "session-1",
        "proj",
        "file_create",
        "Create main",
        None,
        Some(r#"["src/main.rs","src/bin.rs"]"#),
        None,
    )
    .unwrap();
    insert_event(
        &conn,
        "session-1",
        "proj",
        "bash",
        "Run tests",
        None,
        Some("not-json"),
        Some(0),
    )
    .unwrap();

    let files = get_session_files_modified(&conn, "session-1").unwrap();
    assert_eq!(files, vec!["src/lib.rs", "src/main.rs", "src/bin.rs"]);
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
