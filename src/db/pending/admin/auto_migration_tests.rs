use rusqlite::{params, Connection};

use super::auto_migrate_actionable_legacy_pending;
use crate::{db, migrate::MIGRATIONS};

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    for migration in MIGRATIONS {
        conn.execute_batch(migration.sql)
            .expect("schema migration should load");
    }
    conn
}

fn insert_legacy_row(conn: &Connection, session_id: &str, created_at_epoch: i64) -> i64 {
    let id = db::test_support::insert_legacy_pending_fixture(
        conn,
        crate::runtime_config::CODEX_HOST,
        session_id,
        "alpha",
        "tool",
        None,
        None,
        None,
    )
    .expect("legacy fixture should insert");
    conn.execute(
        "UPDATE pending_observations
         SET created_at_epoch = ?2, updated_at_epoch = ?2
         WHERE id = ?1",
        params![id, created_at_epoch],
    )
    .expect("legacy fixture time should update");
    id
}

#[test]
fn auto_migration_handles_pending_and_expired_processing_but_skips_active_processing() {
    let mut conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let pending_id = insert_legacy_row(&conn, "pending", now - 300);
    let expired_id = insert_legacy_row(&conn, "expired", now - 200);
    let active_id = insert_legacy_row(&conn, "active", now - 100);
    conn.execute(
        "UPDATE pending_observations
         SET status = 'processing', attempt_count = 7,
             failure_class = 'transient', failed_at_epoch = ?2,
             archived_at_epoch = ?2, lease_owner = 'dead-worker',
             lease_expires_epoch = ?3
         WHERE id = ?1",
        params![expired_id, now - 500, now - 1],
    )
    .expect("expired processing row should update");
    conn.execute(
        "UPDATE pending_observations
         SET status = 'processing', lease_owner = 'active-worker',
             lease_expires_epoch = ?2
         WHERE id = ?1",
        params![active_id, now + 300],
    )
    .expect("active processing row should update");

    let outcome = auto_migrate_actionable_legacy_pending(&mut conn, 10)
        .expect("eligible rows should migrate");

    assert_eq!(outcome.migrated, 2);
    let states: Vec<(i64, String, i64, Option<i64>)> = conn
        .prepare(
            "SELECT id, status, attempt_count, archived_at_epoch
             FROM pending_observations ORDER BY id",
        )
        .expect("state query should prepare")
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("states should query")
        .collect::<rusqlite::Result<_>>()
        .expect("states should collect");
    assert_eq!(
        states,
        vec![
            (pending_id, "migrated".to_string(), 0, None),
            (expired_id, "migrated".to_string(), 0, None),
            (active_id, "processing".to_string(), 0, None),
        ]
    );
    let (source_updated, event_inserted): (i64, i64) = conn
        .query_row(
            "SELECT p.updated_at_epoch, e.inserted_at_epoch
             FROM pending_observations p
             JOIN captured_events e ON e.event_id = 'legacy-pending-' || p.id
             WHERE p.id = ?1",
            [expired_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("migrated timestamps should query");
    assert!(source_updated >= event_inserted);
}

#[test]
fn auto_migration_commits_prior_rows_and_rolls_back_the_failing_row() {
    let mut conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let first_id = insert_legacy_row(&conn, "first", now - 200);
    let second_id = insert_legacy_row(&conn, "second", now - 100);
    for id in [first_id, second_id] {
        conn.execute(
            "UPDATE pending_observations
             SET status = 'failed', failure_class = 'transient',
                 attempt_count = 3, next_retry_epoch = ?2,
                 failed_at_epoch = ?3, archived_at_epoch = ?4
             WHERE id = ?1",
            params![id, now - 1, now - 500, now - 400],
        )
        .expect("failed state should update");
    }
    conn.execute_batch(&format!(
        "CREATE TRIGGER fail_second_legacy_capture
         BEFORE INSERT ON captured_events
         WHEN NEW.event_id = 'legacy-pending-{second_id}'
         BEGIN
             SELECT RAISE(ABORT, 'injected second-row replay failure');
         END;"
    ))
    .expect("selective capture fault should install");

    let error = auto_migrate_actionable_legacy_pending(&mut conn, 10)
        .expect_err("second row should abort the remaining batch");

    assert!(format!("{error:#}").contains("migrated_before_error=1"));
    let first_state: (String, i64) = conn
        .query_row(
            "SELECT status, attempt_count FROM pending_observations WHERE id = ?1",
            [first_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("first row should query");
    assert_eq!(first_state, ("migrated".to_string(), 0));
    let second_state: (String, i64, i64, i64) = conn
        .query_row(
            "SELECT status, attempt_count, failed_at_epoch, archived_at_epoch
             FROM pending_observations WHERE id = ?1",
            [second_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("second row should query");
    assert_eq!(
        second_state,
        ("failed".to_string(), 4, now - 500, now - 400)
    );
    let captured_ids: Vec<String> = conn
        .prepare("SELECT event_id FROM captured_events ORDER BY event_id")
        .expect("captured-event query should prepare")
        .query_map([], |row| row.get(0))
        .expect("captured events should query")
        .collect::<rusqlite::Result<_>>()
        .expect("captured events should collect");
    assert_eq!(captured_ids, vec![format!("legacy-pending-{first_id}")]);
}
