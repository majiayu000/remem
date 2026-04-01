use rusqlite::{params, Connection};

use super::support::setup_workstream_schema;
use crate::workstream::{
    query_workstreams, update_workstream_manual, upsert_workstream, ParsedWorkStream,
};

#[test]
fn test_skip_when_title_none() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let parsed = ParsedWorkStream {
        title: None,
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    let result = upsert_workstream(&conn, "test/proj", "mem-abc", &parsed);
    assert!(result.is_err());
}

#[test]
fn test_update_workstream_manual() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('test/proj', 'Manual Task', 'active', ?1, ?1)",
        params![now],
    )
    .unwrap();
    let workstream_id = conn.last_insert_rowid();

    let updated = update_workstream_manual(
        &conn,
        workstream_id,
        Some("completed"),
        Some("Ship it"),
        Some("None"),
    )
    .unwrap();
    assert!(updated);

    let completed = query_workstreams(&conn, "test/proj", Some("completed")).unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].next_action.as_deref(), Some("Ship it"));
    assert_eq!(completed[0].blockers.as_deref(), Some("None"));
    assert!(completed[0].completed_at_epoch.is_some());
}

#[test]
fn test_update_workstream_manual_returns_false_when_missing() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let updated = update_workstream_manual(&conn, 999, Some("paused"), None, None).unwrap();
    assert!(!updated);
}
