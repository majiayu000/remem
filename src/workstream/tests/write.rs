use rusqlite::{params, Connection};

use super::support::setup_workstream_schema;
use crate::workstream::{
    merge_workstreams_manual, query_active_workstreams, query_workstreams,
    update_workstream_manual, upsert_workstream, ParsedWorkStream,
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

#[test]
fn manual_merge_moves_sessions_and_aliases_to_canonical_workstream() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let canonical = ParsedWorkStream {
        title: Some("Canonical Workstream".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    let canonical_id = upsert_workstream(&conn, "test/proj", "mem-canonical", &canonical).unwrap();

    let duplicate = ParsedWorkStream {
        title: Some("Duplicate Workstream".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    let duplicate_id = upsert_workstream(&conn, "test/proj", "mem-duplicate", &duplicate).unwrap();
    let renamed_duplicate = ParsedWorkStream {
        title: Some("Renamed Duplicate Workstream".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    upsert_workstream(&conn, "test/proj", "mem-duplicate", &renamed_duplicate).unwrap();

    let result =
        merge_workstreams_manual(&conn, "test/proj", canonical_id, &[duplicate_id]).unwrap();

    assert_eq!(result.canonical_id, canonical_id);
    assert_eq!(result.merged_ids, vec![duplicate_id]);
    assert_eq!(result.moved_session_links, 1);
    assert!(result.copied_aliases >= 2);

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 1);
    assert_eq!(workstreams[0].id, canonical_id);

    let canonical_sessions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workstream_sessions WHERE workstream_id = ?1",
            params![canonical_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(canonical_sessions, 2);
    let duplicate_sessions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workstream_sessions WHERE workstream_id = ?1",
            params![duplicate_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(duplicate_sessions, 0);

    let later_repeat = ParsedWorkStream {
        title: Some("Renamed Duplicate Workstream".to_string()),
        progress: Some("later repeat".to_string()),
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    let later_id = upsert_workstream(&conn, "test/proj", "mem-later", &later_repeat).unwrap();
    assert_eq!(later_id, canonical_id);
}

#[test]
fn manual_merge_rejects_cross_project_duplicates() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let canonical = ParsedWorkStream {
        title: Some("Canonical Workstream".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    let canonical_id = upsert_workstream(&conn, "test/proj", "mem-canonical", &canonical).unwrap();
    let duplicate = ParsedWorkStream {
        title: Some("Duplicate Workstream".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    let duplicate_id = upsert_workstream(&conn, "other/proj", "mem-duplicate", &duplicate).unwrap();

    let error =
        merge_workstreams_manual(&conn, "test/proj", canonical_id, &[duplicate_id]).unwrap_err();
    assert!(error.to_string().contains("project"));
}
