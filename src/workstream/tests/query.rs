use rusqlite::{params, Connection};

use super::support::setup_workstream_schema;
use crate::workstream::{
    find_matching_workstream, query_active_workstreams, query_workstreams, upsert_workstream,
    ParsedWorkStream, WorkStreamStatus,
};

#[test]
fn test_upsert_creates_new() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let parsed = ParsedWorkStream {
        title: Some("Implement WorkStream".to_string()),
        progress: Some("Started design".to_string()),
        next_action: Some("Write code".to_string()),
        blockers: None,
        is_completed: false,
    };
    let id = upsert_workstream(&conn, "test/proj", "mem-abc", &parsed).unwrap();
    assert!(id > 0);

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 1);
    assert_eq!(workstreams[0].title, "Implement WorkStream");
    assert_eq!(workstreams[0].status, WorkStreamStatus::Active);
}

#[test]
fn test_upsert_updates_existing() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let parsed1 = ParsedWorkStream {
        title: Some("Feature X".to_string()),
        progress: Some("Step 1 done".to_string()),
        next_action: Some("Step 2".to_string()),
        blockers: None,
        is_completed: false,
    };
    let id1 = upsert_workstream(&conn, "test/proj", "mem-abc", &parsed1).unwrap();

    let parsed2 = ParsedWorkStream {
        title: Some("Feature X".to_string()),
        progress: Some("Step 2 done".to_string()),
        next_action: Some("Step 3".to_string()),
        blockers: None,
        is_completed: false,
    };
    let id2 = upsert_workstream(&conn, "test/proj", "mem-def", &parsed2).unwrap();
    assert_eq!(id1, id2);

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 1);
    assert_eq!(workstreams[0].progress.as_deref(), Some("Step 2 done"));
}

#[test]
fn test_fuzzy_match() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let parsed = ParsedWorkStream {
        title: Some("WorkStream 层实现".to_string()),
        progress: Some("设计完成".to_string()),
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    upsert_workstream(&conn, "test/proj", "mem-abc", &parsed).unwrap();

    let found = find_matching_workstream(&conn, "test/proj", "WorkStream").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().title, "WorkStream 层实现");
}

#[test]
fn test_no_match_creates_new() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let parsed1 = ParsedWorkStream {
        title: Some("Feature A".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    upsert_workstream(&conn, "test/proj", "mem-abc", &parsed1).unwrap();

    let parsed2 = ParsedWorkStream {
        title: Some("Feature B".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    upsert_workstream(&conn, "test/proj", "mem-def", &parsed2).unwrap();

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 2);
}

#[test]
fn test_only_matches_active_or_paused() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('test/proj', 'Old Task', 'completed', ?1, ?1)",
        params![now],
    )
    .unwrap();

    let found = find_matching_workstream(&conn, "test/proj", "Old Task").unwrap();
    assert!(found.is_none());
}

#[test]
fn test_completed_status() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let parsed = ParsedWorkStream {
        title: Some("Done Task".to_string()),
        progress: Some("All done".to_string()),
        next_action: None,
        blockers: None,
        is_completed: true,
    };
    upsert_workstream(&conn, "test/proj", "mem-abc", &parsed).unwrap();

    let active = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(active.len(), 0);

    let completed = query_workstreams(&conn, "test/proj", Some("completed")).unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].status, WorkStreamStatus::Completed);
    assert!(completed[0].completed_at_epoch.is_some());
}
