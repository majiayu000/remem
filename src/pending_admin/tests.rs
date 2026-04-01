use rusqlite::{params, Connection};

use super::{list_failed, purge_failed, retry_failed};
use crate::{db, migrate::MIGRATIONS};

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    conn.execute_batch(MIGRATIONS[0].sql)
        .expect("baseline schema should load");
    conn
}

fn insert_failed_row(
    conn: &Connection,
    session_id: &str,
    project: &str,
    updated_at_epoch: i64,
    last_error: &str,
) -> i64 {
    let id = db::enqueue_pending(conn, session_id, project, "tool", None, None, None)
        .expect("pending row should enqueue");
    conn.execute(
        "UPDATE pending_observations
         SET status = 'failed',
             attempt_count = 3,
             updated_at_epoch = ?2,
             last_error = ?3,
             lease_owner = 'worker-x',
             lease_expires_epoch = ?2,
             next_retry_epoch = ?2
         WHERE id = ?1",
        params![id, updated_at_epoch, last_error],
    )
    .expect("failed row should update");
    id
}

#[test]
fn list_failed_filters_by_project_and_limit() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let newest = insert_failed_row(&conn, "s-1", "alpha", now - 10, "err-1");
    insert_failed_row(&conn, "s-2", "alpha", now - 20, "err-2");
    insert_failed_row(&conn, "s-3", "beta", now - 5, "err-3");

    let rows = list_failed(&conn, Some("alpha"), 1).expect("failed rows should load");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, newest);
    assert_eq!(rows[0].project, "alpha");
}

#[test]
fn retry_failed_resets_rows_for_selected_project() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let alpha_id = insert_failed_row(&conn, "s-1", "alpha", now - 10, "alpha boom");
    let beta_id = insert_failed_row(&conn, "s-2", "beta", now - 20, "beta boom");

    let changed = retry_failed(&conn, Some("alpha"), 5).expect("retry should succeed");

    assert_eq!(changed, 1);
    let alpha_row = conn
        .query_row(
            "SELECT status, lease_owner, lease_expires_epoch, next_retry_epoch, last_error
             FROM pending_observations WHERE id = ?1",
            params![alpha_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .expect("alpha row should exist");
    assert_eq!(alpha_row.0, "pending");
    assert_eq!(alpha_row.1, None);
    assert_eq!(alpha_row.2, None);
    assert_eq!(alpha_row.3, None);
    assert_eq!(alpha_row.4, None);

    let beta_status: String = conn
        .query_row(
            "SELECT status FROM pending_observations WHERE id = ?1",
            params![beta_id],
            |row| row.get(0),
        )
        .expect("beta row should exist");
    assert_eq!(beta_status, "failed");
}

#[test]
fn purge_failed_respects_cutoff_and_project() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let old_alpha = insert_failed_row(&conn, "s-1", "alpha", now - 5 * 86_400, "old alpha");
    let recent_alpha = insert_failed_row(&conn, "s-2", "alpha", now - 86_400, "recent alpha");
    let old_beta = insert_failed_row(&conn, "s-3", "beta", now - 5 * 86_400, "old beta");

    let changed = purge_failed(&conn, Some("alpha"), 2).expect("purge should succeed");

    assert_eq!(changed, 1);
    let remaining_ids = conn
        .prepare("SELECT id FROM pending_observations ORDER BY id ASC")
        .expect("select should prepare")
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("rows should load")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("rows should collect");
    assert_eq!(remaining_ids, vec![recent_alpha, old_beta]);
    assert!(!remaining_ids.contains(&old_alpha));
}
