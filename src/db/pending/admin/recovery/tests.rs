use super::*;
use std::sync::{mpsc, Arc, Barrier};
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
struct SourceState {
    status: String,
    attempt_count: i64,
    next_retry_epoch: Option<i64>,
    last_error: Option<String>,
    lease_owner: Option<String>,
    lease_expires_epoch: Option<i64>,
    failure_class: Option<String>,
    failed_at_epoch: Option<i64>,
    archived_at_epoch: Option<i64>,
    updated_at_epoch: i64,
}

fn setup_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn seed_archived_failure(
    conn: &Connection,
    host: &str,
    session_id: &str,
    failure_class: &str,
) -> Result<i64> {
    let id = crate::db::test_support::insert_legacy_pending_fixture(
        conn,
        host,
        session_id,
        "/tmp/remem-archived-recovery",
        "Bash",
        Some(r#"{"cmd":"printf recovered"}"#),
        Some(r#"{"output":"recovered"}"#),
        Some("/tmp/remem-archived-recovery"),
    )?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE pending_observations
         SET status = 'failed',
             attempt_count = 7,
             next_retry_epoch = ?2,
             last_error = 'legacy replay failed',
             lease_owner = 'dead-worker',
             lease_expires_epoch = ?3,
             failure_class = ?4,
             failed_at_epoch = ?5,
             archived_at_epoch = ?6,
             updated_at_epoch = ?7
         WHERE id = ?1",
        params![
            id,
            now + 3_600,
            now - 100,
            failure_class,
            now - 2_000,
            now - 1_000,
            now - 900
        ],
    )?;
    Ok(id)
}

fn source_state(conn: &Connection, id: i64) -> Result<SourceState> {
    conn.query_row(
        "SELECT status, attempt_count, next_retry_epoch, last_error, lease_owner,
                lease_expires_epoch, failure_class, failed_at_epoch, archived_at_epoch,
                updated_at_epoch
         FROM pending_observations WHERE id = ?1",
        [id],
        |row| {
            Ok(SourceState {
                status: row.get(0)?,
                attempt_count: row.get(1)?,
                next_retry_epoch: row.get(2)?,
                last_error: row.get(3)?,
                lease_owner: row.get(4)?,
                lease_expires_epoch: row.get(5)?,
                failure_class: row.get(6)?,
                failed_at_epoch: row.get(7)?,
                archived_at_epoch: row.get(8)?,
                updated_at_epoch: row.get(9)?,
            })
        },
    )
    .map_err(Into::into)
}

fn captured_event_count(conn: &Connection, id: i64) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM captured_events WHERE event_id = ?1",
        [format!("legacy-pending-{id}")],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

#[test]
fn dry_run_requires_unknown_host_and_never_writes() -> Result<()> {
    let conn = setup_conn()?;
    let id = seed_archived_failure(&conn, "unknown", "dry-run-unknown", "transient")?;
    let before = source_state(&conn, id)?;

    let error = preview_archived_legacy_pending_recovery(&conn, id, None)
        .expect_err("unknown host preview must require an explicit host");

    assert!(format!("{error:#}").contains("pass --host"));
    let preview = preview_archived_legacy_pending_recovery(
        &conn,
        id,
        Some(crate::runtime_config::CODEX_HOST),
    )?;
    assert_eq!(preview.pending_id, id);
    assert_eq!(preview.stored_host, "unknown");
    assert_eq!(preview.resolved_host, crate::runtime_config::CODEX_HOST);
    assert!(preview.requires_host);
    assert_eq!(source_state(&conn, id)?, before);
    assert_eq!(captured_event_count(&conn, id)?, 0);
    Ok(())
}

#[test]
fn exact_recovery_migrates_known_host_and_clears_legacy_state() -> Result<()> {
    let mut conn = setup_conn()?;
    let target_id = seed_archived_failure(
        &conn,
        crate::runtime_config::CODEX_HOST,
        "known-target",
        "transient",
    )?;
    let sibling_id = seed_archived_failure(
        &conn,
        crate::runtime_config::CODEX_HOST,
        "known-sibling",
        "transient",
    )?;
    let target_before = source_state(&conn, target_id)?;
    let sibling_before = source_state(&conn, sibling_id)?;

    let outcome = recover_archived_legacy_pending(&mut conn, target_id, None)?;

    assert_eq!(outcome.candidate.pending_id, target_id);
    assert!(!outcome.candidate.requires_host);
    assert_eq!(
        outcome.candidate.resolved_host,
        crate::runtime_config::CODEX_HOST
    );
    assert_eq!(outcome.migrated.pending_id, target_id);
    assert_eq!(outcome.migrated.host, crate::runtime_config::CODEX_HOST);
    let target_after = source_state(&conn, target_id)?;
    assert!(target_after.updated_at_epoch > target_before.updated_at_epoch);
    assert_eq!(
        target_after,
        SourceState {
            status: "migrated".to_string(),
            attempt_count: 0,
            next_retry_epoch: None,
            last_error: None,
            lease_owner: None,
            lease_expires_epoch: None,
            failure_class: None,
            failed_at_epoch: None,
            archived_at_epoch: None,
            updated_at_epoch: target_after.updated_at_epoch,
        }
    );
    assert_eq!(source_state(&conn, sibling_id)?, sibling_before);
    assert_eq!(captured_event_count(&conn, target_id)?, 1);
    assert_eq!(captured_event_count(&conn, sibling_id)?, 0);
    Ok(())
}

#[test]
fn unknown_host_requires_explicit_fallback_and_then_recovers() -> Result<()> {
    let mut conn = setup_conn()?;
    let id = seed_archived_failure(&conn, "unknown", "unknown-target", "transient")?;
    let before = source_state(&conn, id)?;

    let error = recover_archived_legacy_pending(&mut conn, id, None)
        .expect_err("unknown host must require explicit fallback");

    assert!(format!("{error:#}").contains("pass --host"));
    assert_eq!(source_state(&conn, id)?, before);
    let outcome =
        recover_archived_legacy_pending(&mut conn, id, Some(crate::runtime_config::CLAUDE_HOST))?;
    assert!(outcome.candidate.requires_host);
    assert_eq!(
        outcome.candidate.resolved_host,
        crate::runtime_config::CLAUDE_HOST
    );
    assert_eq!(outcome.migrated.host, crate::runtime_config::CLAUDE_HOST);
    assert_eq!(source_state(&conn, id)?.status, "migrated");
    assert_eq!(captured_event_count(&conn, id)?, 1);
    Ok(())
}

#[test]
fn explicit_recovery_allows_archived_permanent_rows() -> Result<()> {
    let mut conn = setup_conn()?;
    let id = seed_archived_failure(
        &conn,
        crate::runtime_config::CLAUDE_HOST,
        "permanent-target",
        "permanent",
    )?;

    let outcome = recover_archived_legacy_pending(&mut conn, id, None)?;

    assert_eq!(
        outcome.candidate.failure_class.as_deref(),
        Some("permanent")
    );
    assert_eq!(outcome.migrated.pending_id, id);
    assert_eq!(source_state(&conn, id)?.failure_class, None);
    assert_eq!(source_state(&conn, id)?.archived_at_epoch, None);
    Ok(())
}

#[test]
fn source_transition_failure_rolls_back_replay_and_preserves_every_field() -> Result<()> {
    let mut conn = setup_conn()?;
    let id = seed_archived_failure(
        &conn,
        crate::runtime_config::CODEX_HOST,
        "rollback-target",
        "permanent",
    )?;
    let before = source_state(&conn, id)?;
    conn.execute_batch(&format!(
        "CREATE TRIGGER fail_archived_recovery_source_transition
         BEFORE UPDATE ON pending_observations
         WHEN OLD.id = {id} AND NEW.status = 'migrated'
         BEGIN
             SELECT RAISE(ABORT, 'injected archived recovery transition failure');
         END;"
    ))?;

    let error = recover_archived_legacy_pending(&mut conn, id, None)
        .expect_err("source transition failure must roll back replay");

    assert!(format!("{error:#}").contains("injected archived recovery transition failure"));
    assert_eq!(source_state(&conn, id)?, before);
    assert_eq!(captured_event_count(&conn, id)?, 0);
    let extraction_tasks: i64 =
        conn.query_row("SELECT COUNT(*) FROM extraction_tasks", [], |row| {
            row.get(0)
        })?;
    assert_eq!(extraction_tasks, 0);
    Ok(())
}

#[test]
fn recovery_rejects_missing_non_archived_and_already_recovered_rows() -> Result<()> {
    let mut conn = setup_conn()?;
    let pending_id = crate::db::test_support::insert_legacy_pending_fixture(
        &conn,
        crate::runtime_config::CODEX_HOST,
        "non-archived",
        "/tmp/remem-archived-recovery",
        "Bash",
        None,
        None,
        None,
    )?;
    for id in [i64::MAX, pending_id] {
        let error = recover_archived_legacy_pending(&mut conn, id, None)
            .expect_err("only an existing archived failed row may be recovered");
        assert!(format!("{error:#}").contains("is not recoverable"));
    }

    let archived_id = seed_archived_failure(
        &conn,
        crate::runtime_config::CODEX_HOST,
        "recover-once",
        "permanent",
    )?;
    recover_archived_legacy_pending(&mut conn, archived_id, None)?;
    let error = recover_archived_legacy_pending(&mut conn, archived_id, None)
        .expect_err("a migrated row must not be recovered twice");
    assert!(format!("{error:#}").contains("is not recoverable"));
    assert_eq!(captured_event_count(&conn, archived_id)?, 1);
    let task_count: i64 = conn.query_row("SELECT COUNT(*) FROM extraction_tasks", [], |row| {
        row.get(0)
    })?;
    assert_eq!(task_count, 1);
    Ok(())
}

#[test]
fn concurrent_exact_recovery_commits_once_without_overwriting_terminal_state() -> Result<()> {
    let db_path = crate::db::test_support::unique_temp_db_path("archived-recovery-race");
    let seed_conn = Connection::open(&db_path)?;
    crate::migrate::run_migrations(&seed_conn)?;
    let id = seed_archived_failure(
        &seed_conn,
        crate::runtime_config::CODEX_HOST,
        "concurrent-recovery",
        "permanent",
    )?;
    drop(seed_conn);

    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let db_path = db_path.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || -> Result<bool> {
            let mut conn = Connection::open(db_path)?;
            conn.busy_timeout(Duration::from_secs(5))?;
            barrier.wait();
            match recover_archived_legacy_pending(&mut conn, id, None) {
                Ok(_) => Ok(true),
                Err(error) if format!("{error:#}").contains("is not recoverable") => Ok(false),
                Err(error) => Err(error),
            }
        }));
    }
    barrier.wait();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().expect("recovery thread should not panic"))
        .collect::<Result<Vec<_>>>()?;
    assert_eq!(outcomes.iter().filter(|succeeded| **succeeded).count(), 1);

    let conn = Connection::open(&db_path)?;
    assert_eq!(source_state(&conn, id)?.status, "migrated");
    assert_eq!(captured_event_count(&conn, id)?, 1);
    let task_count: i64 = conn.query_row("SELECT COUNT(*) FROM extraction_tasks", [], |row| {
        row.get(0)
    })?;
    assert_eq!(task_count, 1);
    drop(conn);
    crate::db::test_support::cleanup_temp_db_files(&db_path);
    Ok(())
}

#[test]
fn exact_detector_runs_without_writer_lock_and_snapshot_drift_fails_explicitly() -> Result<()> {
    let db_path = crate::db::test_support::unique_temp_db_path("exact-detector-lock");
    let seed_conn = Connection::open(&db_path)?;
    crate::migrate::run_migrations(&seed_conn)?;
    let id = seed_archived_failure(
        &seed_conn,
        crate::runtime_config::CODEX_HOST,
        "exact-detector",
        "permanent",
    )?;
    drop(seed_conn);

    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let worker_path = db_path.clone();
    let handle = std::thread::spawn(move || {
        let result = (|| -> Result<ArchivedLegacyPendingRecovery> {
            let mut conn = Connection::open(worker_path)?;
            conn.busy_timeout(Duration::from_secs(5))?;
            let mut detector = move |_cwd: &str| {
                entered_tx.send(()).expect("announce exact detector entry");
                resume_rx.recv().expect("resume exact detector");
                None
            };
            recover_archived_legacy_pending_with_detector(&mut conn, id, None, &mut detector)
        })();
        done_tx
            .send(result)
            .expect("publish exact recovery completion");
    });

    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("exact detector should run within timeout");
    let observer = Connection::open(&db_path)?;
    observer.busy_timeout(Duration::from_millis(100))?;
    let lock_probe = observer.execute_batch("BEGIN IMMEDIATE; ROLLBACK;");
    let changed_response = r#"{"output":"changed during exact preflight"}"#;
    let drift = observer.execute(
        "UPDATE pending_observations
         SET tool_response = ?2,
             attempt_count = 8,
             last_error = 'changed during exact preflight',
             updated_at_epoch = ?3
         WHERE id = ?1",
        params![id, changed_response, chrono::Utc::now().timestamp()],
    );
    resume_tx.send(()).expect("resume exact recovery");
    let result = done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("exact recovery should complete within timeout");
    handle.join().expect("exact recovery thread should join");
    lock_probe?;
    drift?;

    let error = result.expect_err("exact snapshot drift must fail explicitly");
    assert!(format!("{error:#}").contains("changed while preparing recovery"));
    assert_eq!(source_state(&observer, id)?.status, "failed");
    let state: (String, i64, Option<String>) = observer.query_row(
        "SELECT tool_response, attempt_count, last_error
         FROM pending_observations
         WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(
        state,
        (
            changed_response.to_string(),
            8,
            Some("changed during exact preflight".to_string())
        )
    );
    assert_eq!(captured_event_count(&observer, id)?, 0);
    let task_count: i64 =
        observer.query_row("SELECT COUNT(*) FROM extraction_tasks", [], |row| {
            row.get(0)
        })?;
    assert_eq!(task_count, 0);
    drop(observer);
    crate::db::test_support::cleanup_temp_db_files(&db_path);
    Ok(())
}
