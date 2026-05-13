use rusqlite::{params, Connection};

use super::queue::MAX_PENDING_FIELD_BYTES;
use super::{
    claim_pending, delete_pending_claimed, enqueue_pending, release_expired_pending_claims,
    retry_pending_claimed,
};
use crate::migrate::MIGRATIONS;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    for migration in MIGRATIONS {
        conn.execute_batch(migration.sql)
            .expect("schema migration should load");
    }
    conn
}

#[test]
fn claim_pending_only_returns_requested_session_rows() {
    let conn = setup_conn();
    let expected_id = enqueue_pending(
        &conn,
        "codex-cli",
        "session-a",
        "proj",
        "tool",
        None,
        None,
        None,
    )
    .expect("first pending row should be queued");
    enqueue_pending(
        &conn,
        "codex-cli",
        "session-b",
        "proj",
        "tool",
        None,
        None,
        None,
    )
    .expect("second pending row should be queued");

    let claimed = claim_pending(&conn, "codex-cli", "proj", "session-a", 5, "worker-a", 60)
        .expect("session-specific claim should succeed");

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, expected_id);
    assert_eq!(claimed[0].session_id, "session-a");
    assert_eq!(claimed[0].status, "processing");
}

#[test]
fn enqueue_pending_bounds_large_tool_payloads() {
    let conn = setup_conn();
    let oversized_input = "input ".repeat(MAX_PENDING_FIELD_BYTES);
    let oversized_response = "响应".repeat(MAX_PENDING_FIELD_BYTES);

    let id = enqueue_pending(
        &conn,
        "claude-code",
        "session-a",
        "proj",
        "Edit",
        Some(&oversized_input),
        Some(&oversized_response),
        Some("/tmp/proj"),
    )
    .expect("large pending row should be queued");

    let (stored_input, stored_response): (String, String) = conn
        .query_row(
            "SELECT tool_input, tool_response FROM pending_observations WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("stored payload should load");

    assert!(stored_input.len() <= MAX_PENDING_FIELD_BYTES);
    assert!(stored_response.len() <= MAX_PENDING_FIELD_BYTES);
    assert!(stored_input.contains("remem truncated legacy pending field"));
    assert!(stored_response.contains("remem truncated legacy pending field"));
}

#[test]
fn claim_pending_respects_host_project_session_identity() {
    let conn = setup_conn();
    let expected_id = enqueue_pending(
        &conn,
        "codex-cli",
        "session-a",
        "proj-a",
        "tool",
        None,
        None,
        None,
    )
    .expect("target row should be queued");
    enqueue_pending(
        &conn,
        "claude-code",
        "session-a",
        "proj-a",
        "tool",
        None,
        None,
        None,
    )
    .expect("different host row should be queued");
    enqueue_pending(
        &conn,
        "codex-cli",
        "session-a",
        "proj-b",
        "tool",
        None,
        None,
        None,
    )
    .expect("different project row should be queued");

    let claimed = claim_pending(
        &conn,
        "codex-cli",
        "proj-a",
        "session-a",
        10,
        "worker-a",
        60,
    )
    .expect("identity claim should succeed");

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, expected_id);
    assert_eq!(claimed[0].host, "codex-cli");
    assert_eq!(claimed[0].project, "proj-a");
    assert_eq!(claimed[0].session_id, "session-a");
}

#[test]
fn claim_pending_allows_legacy_unknown_host_for_matching_identity() {
    let conn = setup_conn();
    let legacy_id = enqueue_pending(
        &conn,
        "unknown",
        "session-a",
        "proj-a",
        "tool",
        None,
        None,
        None,
    )
    .expect("legacy row should be queued");
    enqueue_pending(
        &conn,
        "unknown",
        "session-a",
        "proj-b",
        "tool",
        None,
        None,
        None,
    )
    .expect("different legacy project row should be queued");

    let claimed = claim_pending(
        &conn,
        "codex-cli",
        "proj-a",
        "session-a",
        10,
        "worker-a",
        60,
    )
    .expect("legacy-compatible claim should succeed");

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, legacy_id);
    assert_eq!(claimed[0].host, "unknown");
}

#[test]
fn retry_pending_claimed_resets_status_and_sets_next_retry() {
    let conn = setup_conn();
    let pending_id = enqueue_pending(
        &conn,
        "codex-cli",
        "session-a",
        "proj",
        "tool",
        None,
        None,
        None,
    )
    .expect("pending row should be queued");
    let claimed = claim_pending(&conn, "codex-cli", "proj", "session-a", 1, "worker-a", 60)
        .expect("pending row should be claimed");
    let lower_bound = chrono::Utc::now().timestamp() + 120;

    let retried = retry_pending_claimed(&conn, "worker-a", &[claimed[0].id], "boom", 120)
        .expect("retry should succeed");

    assert_eq!(retried, 1);
    let row = conn
        .query_row(
            "SELECT status, lease_owner, next_retry_epoch, last_error, attempt_count
             FROM pending_observations WHERE id = ?1",
            params![pending_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .expect("retried row should exist");

    assert_eq!(row.0, "pending");
    assert_eq!(row.1, None);
    assert_eq!(row.3.as_deref(), Some("boom"));
    assert_eq!(row.4, 1);
    assert!(row.2.is_some());
    assert!(row.2.expect("next retry should be set") >= lower_bound);
}

#[test]
fn delete_pending_claimed_only_deletes_processing_rows_for_owner() {
    let conn = setup_conn();
    let owned_id = enqueue_pending(
        &conn,
        "codex-cli",
        "session-a",
        "proj",
        "tool",
        None,
        None,
        None,
    )
    .expect("owned row should be queued");
    let pending_id = enqueue_pending(
        &conn,
        "codex-cli",
        "session-a",
        "proj",
        "tool",
        None,
        None,
        None,
    )
    .expect("unclaimed row should be queued");
    let other_owner_id = enqueue_pending(
        &conn,
        "codex-cli",
        "session-b",
        "proj",
        "tool",
        None,
        None,
        None,
    )
    .expect("other owner row should be queued");

    claim_pending(&conn, "codex-cli", "proj", "session-a", 1, "worker-a", 60)
        .expect("worker a should claim its row");
    claim_pending(&conn, "codex-cli", "proj", "session-b", 1, "worker-b", 60)
        .expect("worker b should claim its row");

    let deleted =
        delete_pending_claimed(&conn, "worker-a", &[owned_id, pending_id, other_owner_id])
            .expect("delete should succeed");

    assert_eq!(deleted, 1);
    let remaining_ids = conn
        .prepare("SELECT id FROM pending_observations ORDER BY id ASC")
        .expect("select should prepare")
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("rows should load")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("row collection should succeed");
    assert_eq!(remaining_ids, vec![pending_id, other_owner_id]);
}

#[test]
fn release_expired_pending_claims_resets_only_expired_processing_rows() {
    let conn = setup_conn();
    let expired_id = enqueue_pending(
        &conn,
        "codex-cli",
        "session-a",
        "proj",
        "tool",
        None,
        None,
        None,
    )
    .expect("expired row should be queued");
    let fresh_id = enqueue_pending(
        &conn,
        "codex-cli",
        "session-b",
        "proj",
        "tool",
        None,
        None,
        None,
    )
    .expect("fresh row should be queued");
    let pending_id = enqueue_pending(
        &conn,
        "codex-cli",
        "session-c",
        "proj",
        "tool",
        None,
        None,
        None,
    )
    .expect("pending row should be queued");
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE pending_observations
         SET status = 'processing', lease_owner = 'old-worker', lease_expires_epoch = ?2
         WHERE id = ?1",
        params![expired_id, now - 1],
    )
    .expect("expired row should update");
    conn.execute(
        "UPDATE pending_observations
         SET status = 'processing', lease_owner = 'fresh-worker', lease_expires_epoch = ?2
         WHERE id = ?1",
        params![fresh_id, now + 60],
    )
    .expect("fresh row should update");

    let released = release_expired_pending_claims(&conn).expect("release should succeed");

    assert_eq!(released, 1);
    let rows = conn
        .prepare(
            "SELECT id, status, lease_owner
             FROM pending_observations
             ORDER BY id ASC",
        )
        .expect("select should prepare")
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .expect("rows should load")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("rows should collect");

    assert_eq!(
        rows,
        vec![
            (expired_id, "pending".to_string(), None),
            (
                fresh_id,
                "processing".to_string(),
                Some("fresh-worker".to_string())
            ),
            (pending_id, "pending".to_string(), None),
        ]
    );
}
