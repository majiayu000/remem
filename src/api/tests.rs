use axum::{body::to_bytes, extract::State, http::StatusCode, response::IntoResponse};
use rusqlite::params;
use serde_json::Value;

use crate::db::test_support::ScopedTestDataDir;
use crate::{db, memory};

use super::handlers::handle_status;
use super::DbState;

#[test]
fn db_state_is_stateless() {
    assert_eq!(std::mem::size_of::<DbState>(), 0);
}

#[tokio::test]
async fn status_handler_reopens_database_after_file_removal() {
    let test_dir = ScopedTestDataDir::new("api-status");

    let first = handle_status(State(DbState)).await.into_response();
    assert_eq!(first.status(), StatusCode::OK);
    assert!(test_dir.db_path().exists());

    test_dir.remove_db_files();
    assert!(!test_dir.db_path().exists());

    let second = handle_status(State(DbState)).await.into_response();
    assert_eq!(second.status(), StatusCode::OK);
    assert!(test_dir.db_path().exists());
}

#[tokio::test]
async fn status_handler_matches_shared_system_stats() {
    let _test_dir = ScopedTestDataDir::new("api-status-stats");
    let conn = db::open_db().expect("db should open");

    memory::insert_memory(
        &conn,
        Some("session-1"),
        "proj-a",
        None,
        "active memory",
        "kept",
        "decision",
        None,
    )
    .expect("active memory insert should succeed");
    let archived_memory_id = memory::insert_memory(
        &conn,
        Some("session-2"),
        "proj-a",
        None,
        "archived memory",
        "hidden",
        "decision",
        None,
    )
    .expect("archived memory insert should succeed");
    conn.execute(
        "UPDATE memories SET status = 'archived' WHERE id = ?1",
        params![archived_memory_id],
    )
    .expect("memory archive update should succeed");

    db::insert_observation(
        &conn,
        "session-1",
        "proj-a",
        "feature",
        Some("active observation"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        1,
    )
    .expect("active observation insert should succeed");
    let stale_observation_id = db::insert_observation(
        &conn,
        "session-2",
        "proj-a",
        "feature",
        Some("stale observation"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        1,
    )
    .expect("stale observation insert should succeed");
    conn.execute(
        "UPDATE observations SET status = 'stale' WHERE id = ?1",
        params![stale_observation_id],
    )
    .expect("observation stale update should succeed");

    let stats = db::query_system_stats(&conn).expect("system stats should load");
    drop(conn);

    let response = handle_status(State(DbState)).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should read");
    let payload: Value =
        serde_json::from_slice(&body).expect("status response should be valid json");
    assert_eq!(payload["memories"], stats.active_memories);
    assert_eq!(payload["observations"], stats.active_observations);
}
