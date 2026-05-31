use rusqlite::{params, Connection};

use super::support::setup_workstream_schema;
use crate::workstream::{
    auto_abandon_inactive, auto_pause_inactive, query_workstreams, DEFAULT_AUTO_PAUSE_DAYS,
};

#[test]
fn test_auto_pause_after_14_days() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let old_epoch = chrono::Utc::now().timestamp() - ((DEFAULT_AUTO_PAUSE_DAYS + 1) * 86400);
    conn.execute(
        "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('test/proj', 'Stale Task', 'active', ?1, ?1)",
        params![old_epoch],
    )
    .unwrap();

    let paused = auto_pause_inactive(&conn, "test/proj", DEFAULT_AUTO_PAUSE_DAYS).unwrap();
    assert_eq!(paused, 1);

    let workstreams = query_workstreams(&conn, "test/proj", Some("paused")).unwrap();
    assert_eq!(workstreams.len(), 1);
}

#[test]
fn test_auto_abandon_after_30_days() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let old_epoch = chrono::Utc::now().timestamp() - (31 * 86400);
    conn.execute(
        "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('test/proj', 'Very Stale', 'paused', ?1, ?1)",
        params![old_epoch],
    )
    .unwrap();

    let abandoned = auto_abandon_inactive(&conn, "test/proj", 30).unwrap();
    assert_eq!(abandoned, 1);

    let workstreams = query_workstreams(&conn, "test/proj", Some("abandoned")).unwrap();
    assert_eq!(workstreams.len(), 1);
}

#[test]
fn test_auto_pause_skips_recent() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let recent = chrono::Utc::now().timestamp() - ((DEFAULT_AUTO_PAUSE_DAYS - 1) * 86400);
    conn.execute(
        "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('test/proj', 'Recent Task', 'active', ?1, ?1)",
        params![recent],
    )
    .unwrap();

    let paused = auto_pause_inactive(&conn, "test/proj", DEFAULT_AUTO_PAUSE_DAYS).unwrap();
    assert_eq!(paused, 0);
}

#[test]
fn test_auto_pause_uses_repo_owner_metadata() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let old_epoch = chrono::Utc::now().timestamp() - ((DEFAULT_AUTO_PAUSE_DAYS + 1) * 86400);
    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch,
          owner_scope, owner_key, target_project)
         VALUES ('legacy/path', 'Owned Task', 'active', ?1, ?1,
                 'repo', 'test/proj', 'test/proj')",
        params![old_epoch],
    )
    .unwrap();

    let paused = auto_pause_inactive(&conn, "test/proj", DEFAULT_AUTO_PAUSE_DAYS).unwrap();
    assert_eq!(paused, 1);
}

#[test]
fn test_auto_pause_uses_workstream_owner_target_project() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let old_epoch = chrono::Utc::now().timestamp() - ((DEFAULT_AUTO_PAUSE_DAYS + 1) * 86400);
    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch,
          owner_scope, owner_key, target_project)
         VALUES ('legacy/path', 'Owned Workstream', 'active', ?1, ?1,
                 'workstream', 'ws-123', 'test/proj')",
        params![old_epoch],
    )
    .unwrap();

    let paused = auto_pause_inactive(&conn, "test/proj", DEFAULT_AUTO_PAUSE_DAYS).unwrap();
    assert_eq!(paused, 1);
}

#[test]
fn test_cleanup_sequence_can_abandon_more_than_30_days_inactive() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let old_epoch = chrono::Utc::now().timestamp() - (45 * 86400);
    conn.execute(
        "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('test/proj', 'Very Old Active', 'active', ?1, ?1)",
        params![old_epoch],
    )
    .unwrap();

    let paused = auto_pause_inactive(&conn, "test/proj", DEFAULT_AUTO_PAUSE_DAYS).unwrap();
    assert_eq!(paused, 1);
    let abandoned = auto_abandon_inactive(&conn, "test/proj", 30).unwrap();
    assert_eq!(abandoned, 1);

    let workstreams = query_workstreams(&conn, "test/proj", Some("abandoned")).unwrap();
    assert_eq!(workstreams.len(), 1);
}

#[test]
fn test_auto_abandon_skips_active() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let old_epoch = chrono::Utc::now().timestamp() - (31 * 86400);
    conn.execute(
        "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('test/proj', 'Old Active', 'active', ?1, ?1)",
        params![old_epoch],
    )
    .unwrap();

    let abandoned = auto_abandon_inactive(&conn, "test/proj", 30).unwrap();
    assert_eq!(abandoned, 0);
}
