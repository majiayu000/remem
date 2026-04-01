use rusqlite::{params, Connection};

use super::{claim_pending, delete_pending_claimed, enqueue_pending, retry_pending_claimed};
use crate::migrate::MIGRATIONS;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    conn.execute_batch(MIGRATIONS[0].sql)
        .expect("baseline schema should load");
    conn
}

#[test]
fn claim_pending_only_returns_requested_session_rows() {
    let conn = setup_conn();
    let expected_id = enqueue_pending(&conn, "session-a", "proj", "tool", None, None, None)
        .expect("first pending row should be queued");
    enqueue_pending(&conn, "session-b", "proj", "tool", None, None, None)
        .expect("second pending row should be queued");

    let claimed = claim_pending(&conn, "session-a", 5, "worker-a", 60)
        .expect("session-specific claim should succeed");

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, expected_id);
    assert_eq!(claimed[0].session_id, "session-a");
    assert_eq!(claimed[0].status, "processing");
}

#[test]
fn retry_pending_claimed_resets_status_and_sets_next_retry() {
    let conn = setup_conn();
    let pending_id = enqueue_pending(&conn, "session-a", "proj", "tool", None, None, None)
        .expect("pending row should be queued");
    let claimed = claim_pending(&conn, "session-a", 1, "worker-a", 60)
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
    let owned_id = enqueue_pending(&conn, "session-a", "proj", "tool", None, None, None)
        .expect("owned row should be queued");
    let pending_id = enqueue_pending(&conn, "session-a", "proj", "tool", None, None, None)
        .expect("unclaimed row should be queued");
    let other_owner_id = enqueue_pending(&conn, "session-b", "proj", "tool", None, None, None)
        .expect("other owner row should be queued");

    claim_pending(&conn, "session-a", 1, "worker-a", 60).expect("worker a should claim its row");
    claim_pending(&conn, "session-b", 1, "worker-b", 60).expect("worker b should claim its row");

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
