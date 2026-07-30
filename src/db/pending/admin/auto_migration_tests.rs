use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::mpsc;
use std::time::Duration;

use super::{auto_migrate_actionable_legacy_pending, AutoLegacyMigrationOutcome};
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

#[test]
fn auto_migration_yields_when_current_extraction_work_appears_between_rows() {
    let mut conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let first_id = insert_legacy_row(&conn, "first-yield", now - 200);
    let second_id = insert_legacy_row(&conn, "second-yield", now - 100);
    conn.execute(
        "UPDATE pending_observations
         SET cwd = '/tmp/remem-auto-yield'
         WHERE id IN (?1, ?2)",
        params![first_id, second_id],
    )
    .expect("legacy rows should have probeable cwd");
    conn.execute_batch(&format!(
        "CREATE TRIGGER enqueue_current_work_after_first_legacy
         AFTER UPDATE OF status ON pending_observations
         WHEN NEW.id = {first_id} AND NEW.status = 'migrated'
         BEGIN
             INSERT INTO extraction_tasks(
                 task_kind, host_id, workspace_id, project_id, session_row_id,
                 priority, status, idempotency_key, cursor_event_id,
                 high_watermark_event_id, attempts, next_retry_epoch,
                 lease_owner, lease_expires_epoch, last_error,
                 created_at_epoch, updated_at_epoch
             )
             SELECT task_kind, host_id, workspace_id, project_id, session_row_id,
                    priority, 'pending', 'current-work-between-legacy-rows',
                    NULL, high_watermark_event_id, 0, NULL, NULL, NULL, NULL,
                    NEW.updated_at_epoch, NEW.updated_at_epoch
             FROM extraction_tasks
             WHERE id = (
                 SELECT MAX(id)
                 FROM extraction_tasks
             );
         END;"
    ))
    .expect("current-work injection trigger should install");
    let mut detector_calls = 0;
    let mut detector = |_cwd: &str| {
        detector_calls += 1;
        None
    };

    let outcome = super::migration::auto_migrate_actionable_legacy_pending_with_detector(
        &mut conn,
        10,
        &mut detector,
    )
    .expect("first eligible row should migrate");

    assert_eq!(outcome.migrated, 1);
    assert_eq!(
        detector_calls, 1,
        "new current work must stop the batch before another Git probe"
    );
    let states: Vec<(i64, String)> = conn
        .prepare(
            "SELECT id, status
             FROM pending_observations
             WHERE id IN (?1, ?2)
             ORDER BY id",
        )
        .expect("legacy states should prepare")
        .query_map(params![first_id, second_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("legacy states should query")
        .collect::<rusqlite::Result<_>>()
        .expect("legacy states should collect");
    assert_eq!(
        states,
        vec![
            (first_id, "migrated".to_string()),
            (second_id, "pending".to_string())
        ]
    );
}

#[test]
fn auto_migration_yields_when_legacy_task_starts_processing_between_rows() {
    let mut conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let first_id = insert_legacy_row(&conn, "first-processing", now - 200);
    let second_id = insert_legacy_row(&conn, "second-processing", now - 100);
    conn.execute(
        "UPDATE pending_observations
         SET cwd = '/tmp/remem-auto-processing'
         WHERE id IN (?1, ?2)",
        params![first_id, second_id],
    )
    .expect("legacy rows should have probeable cwd");
    conn.execute_batch(&format!(
        "CREATE TRIGGER claim_first_legacy_task
         AFTER UPDATE OF status ON pending_observations
         WHEN NEW.id = {first_id} AND NEW.status = 'migrated'
         BEGIN
             UPDATE extraction_tasks
             SET status = 'processing',
                 lease_owner = 'current-worker',
                 lease_expires_epoch = NEW.updated_at_epoch + 300
             WHERE id = (
                 SELECT MAX(id)
                 FROM extraction_tasks
             );
         END;"
    ))
    .expect("processing transition trigger should install");
    let mut detector_calls = 0;
    let mut detector = |_cwd: &str| {
        detector_calls += 1;
        None
    };

    let outcome = super::migration::auto_migrate_actionable_legacy_pending_with_detector(
        &mut conn,
        10,
        &mut detector,
    )
    .expect("first eligible row should migrate");

    assert_eq!(outcome.migrated, 1);
    assert_eq!(detector_calls, 1);
    let second_state: String = conn
        .query_row(
            "SELECT status FROM pending_observations WHERE id = ?1",
            [second_id],
            |row| row.get(0),
        )
        .expect("second legacy row should exist");
    assert_eq!(second_state, "pending");
}

#[test]
fn auto_migration_yields_when_legacy_task_retry_becomes_due_between_rows() {
    let mut conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let first_id = insert_legacy_row(&conn, "first-retry-due", now - 200);
    let second_id = insert_legacy_row(&conn, "second-retry-due", now - 100);
    conn.execute(
        "UPDATE pending_observations
         SET cwd = '/tmp/remem-auto-retry-due'
         WHERE id IN (?1, ?2)",
        params![first_id, second_id],
    )
    .expect("legacy rows should have probeable cwd");
    conn.execute_batch(&format!(
        "CREATE TRIGGER defer_first_legacy_task_until_now
         AFTER UPDATE OF status ON pending_observations
         WHEN NEW.id = {first_id} AND NEW.status = 'migrated'
         BEGIN
             UPDATE extraction_tasks
             SET next_retry_epoch = NEW.updated_at_epoch
             WHERE id = (
                 SELECT MAX(id)
                 FROM extraction_tasks
             );
         END;"
    ))
    .expect("retry transition trigger should install");
    let mut detector_calls = 0;
    let mut detector = |_cwd: &str| {
        detector_calls += 1;
        None
    };

    let outcome = super::migration::auto_migrate_actionable_legacy_pending_with_detector(
        &mut conn,
        10,
        &mut detector,
    )
    .expect("first eligible row should migrate");

    assert_eq!(outcome.migrated, 1);
    assert_eq!(detector_calls, 1);
    let second_state: String = conn
        .query_row(
            "SELECT status FROM pending_observations WHERE id = ?1",
            [second_id],
            |row| row.get(0),
        )
        .expect("second legacy row should exist");
    assert_eq!(second_state, "pending");
}

#[test]
fn auto_detector_runs_without_writer_lock_and_changed_snapshot_is_skipped() -> Result<()> {
    let db_path = db::test_support::unique_temp_db_path("auto-detector-lock");
    let seed_conn = Connection::open(&db_path)?;
    for migration in MIGRATIONS {
        seed_conn.execute_batch(migration.sql)?;
    }
    let id = insert_legacy_row(
        &seed_conn,
        "auto-detector",
        chrono::Utc::now().timestamp() - 60,
    );
    seed_conn.execute(
        "UPDATE pending_observations
         SET cwd = '/tmp/remem-auto-detector', tool_response = ?2
         WHERE id = ?1",
        params![id, r#"{"output":"before"}"#],
    )?;
    drop(seed_conn);

    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let worker_path = db_path.clone();
    let handle = std::thread::spawn(move || {
        let result = (|| -> Result<AutoLegacyMigrationOutcome> {
            let mut conn = Connection::open(worker_path)?;
            conn.busy_timeout(Duration::from_secs(5))?;
            let mut detector = move |_cwd: &str| {
                entered_tx.send(()).expect("announce auto detector entry");
                resume_rx.recv().expect("resume auto detector");
                None
            };
            super::migration::auto_migrate_actionable_legacy_pending_with_detector(
                &mut conn,
                1,
                &mut detector,
            )
        })();
        done_tx
            .send(result)
            .expect("publish auto migration completion");
    });

    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("auto detector should run within timeout");
    let observer = Connection::open(&db_path)?;
    observer.busy_timeout(Duration::from_millis(100))?;
    let lock_probe = observer.execute_batch("BEGIN IMMEDIATE; ROLLBACK;");
    let changed_response = r#"{"output":"changed during auto preflight"}"#;
    let drift = observer.execute(
        "UPDATE pending_observations
         SET tool_response = ?2,
             attempt_count = 7,
             last_error = 'changed during auto preflight',
             updated_at_epoch = ?3
         WHERE id = ?1",
        params![id, changed_response, chrono::Utc::now().timestamp()],
    );
    resume_tx.send(()).expect("resume auto migration");
    let outcome = done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("auto migration should complete within timeout")?;
    handle.join().expect("auto migration thread should join");
    lock_probe?;
    drift?;

    assert_eq!(outcome.migrated, 0);
    let state: (String, String, i64, Option<String>) = observer.query_row(
        "SELECT status, tool_response, attempt_count, last_error
         FROM pending_observations
         WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        state,
        (
            "pending".to_string(),
            changed_response.to_string(),
            7,
            Some("changed during auto preflight".to_string())
        )
    );
    let counts: (i64, i64) = observer.query_row(
        "SELECT
             (SELECT COUNT(*) FROM captured_events),
             (SELECT COUNT(*) FROM extraction_tasks)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(counts, (0, 0));
    drop(observer);
    db::test_support::cleanup_temp_db_files(&db_path);
    Ok(())
}

#[test]
fn auto_migration_yields_when_current_work_arrives_during_git_probe() -> Result<()> {
    let db_path = db::test_support::unique_temp_db_path("auto-detector-current-work");
    let seed_conn = Connection::open(&db_path)?;
    for migration in MIGRATIONS {
        seed_conn.execute_batch(migration.sql)?;
    }
    let id = insert_legacy_row(
        &seed_conn,
        "auto-current-work",
        chrono::Utc::now().timestamp() - 60,
    );
    seed_conn.execute(
        "UPDATE pending_observations
         SET cwd = '/tmp/remem-auto-current-work'
         WHERE id = ?1",
        [id],
    )?;
    drop(seed_conn);

    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    let worker_path = db_path.clone();
    let handle = std::thread::spawn(move || {
        let mut conn = Connection::open(worker_path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let mut detector = move |_cwd: &str| {
            entered_tx.send(()).expect("announce detector entry");
            resume_rx.recv().expect("resume detector");
            None
        };
        super::migration::auto_migrate_actionable_legacy_pending_with_detector(
            &mut conn,
            1,
            &mut detector,
        )
    });

    entered_rx.recv_timeout(Duration::from_secs(5))?;
    let observer = Connection::open(&db_path)?;
    observer.busy_timeout(Duration::from_secs(5))?;
    db::record_captured_event(
        &observer,
        &db::CaptureEventInput {
            host: crate::runtime_config::CODEX_HOST,
            session_id: "current-during-git-probe",
            project: "alpha",
            cwd: None,
            event_type: "message",
            role: Some("user"),
            tool_name: None,
            content: "current capture must win",
            task_kind: Some(db::ExtractionTaskKind::ObservationExtract),
        },
    )?;
    resume_tx.send(())?;
    let outcome = handle.join().expect("auto migration thread should join")?;

    assert_eq!(outcome.migrated, 0);
    let state: String = observer.query_row(
        "SELECT status FROM pending_observations WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;
    assert_eq!(state, "pending");
    let counts: (i64, i64) = observer.query_row(
        "SELECT
             (SELECT COUNT(*) FROM captured_events
              WHERE event_id = ?1),
             (SELECT COUNT(*) FROM extraction_tasks
              WHERE status = 'pending')",
        params![format!("legacy-pending-{id}")],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(counts, (0, 1));
    drop(observer);
    db::test_support::cleanup_temp_db_files(&db_path);
    Ok(())
}
