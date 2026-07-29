use super::*;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn unknown_host_count_is_queryable() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "CREATE TABLE pending_observations (
            id INTEGER PRIMARY KEY,
            host TEXT NOT NULL,
            session_id TEXT NOT NULL,
            project TEXT NOT NULL,
            tool_name TEXT NOT NULL,
            tool_input TEXT,
            tool_response TEXT,
            cwd TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            status TEXT NOT NULL,
            attempt_count INTEGER NOT NULL,
            next_retry_epoch INTEGER,
            last_error TEXT,
            lease_owner TEXT,
            lease_expires_epoch INTEGER
        );",
    )?;
    conn.execute(
        "INSERT INTO pending_observations
         (host, session_id, project, tool_name, created_at_epoch, updated_at_epoch, status, attempt_count)
         VALUES ('unknown', 's', 'p', 'Edit', 1, 1, 'pending', 0)",
        [],
    )?;

    assert_eq!(count_legacy_migration_candidates(&conn, Some("p"), 10)?, 1);
    assert_eq!(
        count_legacy_migration_candidates(&conn, Some("other"), 10)?,
        0
    );
    Ok(())
}

#[test]
fn legacy_event_id_is_stable() {
    assert_eq!(legacy_event_id(42), "legacy-pending-42");
}

#[test]
fn retry_backoff_caps_without_rewriting_failure_or_archive_time() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let now = chrono::Utc::now().timestamp();
    let failed_at = now - 1_000;
    let archived_at = now - 500;
    let id = crate::db::test_support::insert_legacy_pending_fixture(
        &conn,
        crate::runtime_config::CODEX_HOST,
        "s-capped-retry",
        "alpha",
        "tool",
        None,
        None,
        None,
    )?;
    conn.execute(
        "UPDATE pending_observations
         SET status = 'failed', failure_class = 'transient', attempt_count = 20,
             next_retry_epoch = ?2, failed_at_epoch = ?3, archived_at_epoch = ?4
         WHERE id = ?1",
        params![id, now - 1, failed_at, archived_at],
    )?;

    let retry = mark_legacy_row_for_transient_retry(&conn, id, now, "shared failure")?
        .ok_or_else(|| anyhow::anyhow!("retry transition should update the row"))?;

    assert_eq!(retry.attempt_count, 21);
    assert_eq!(retry.backoff_secs, AUTO_MIGRATION_RETRY_MAX_SECS);
    let state: (i64, i64, i64) = conn.query_row(
        "SELECT failed_at_epoch, archived_at_epoch, next_retry_epoch
         FROM pending_observations WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(
        state,
        (failed_at, archived_at, now + AUTO_MIGRATION_RETRY_MAX_SECS)
    );
    Ok(())
}

#[test]
fn archived_counts_separate_auto_recovery_from_admin_recovery() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let now = chrono::Utc::now().timestamp();
    let insert_archived = |host: &str, failure_class: &str| -> Result<()> {
        let id = crate::db::test_support::insert_legacy_pending_fixture(
            &conn, host, "s", "alpha", "tool", None, None, None,
        )?;
        conn.execute(
            "UPDATE pending_observations
             SET status = 'failed', failure_class = ?2, archived_at_epoch = ?3
             WHERE id = ?1",
            params![id, failure_class, now - 1],
        )?;
        Ok(())
    };
    insert_archived(crate::runtime_config::CODEX_HOST, "transient")?;
    insert_archived(crate::runtime_config::CODEX_HOST, "permanent")?;
    insert_archived("unknown", "transient")?;

    assert_eq!(count_recoverable_archived_legacy_pending(&conn)?, 1);
    assert_eq!(count_admin_required_archived_legacy_pending(&conn)?, 2);
    Ok(())
}

#[test]
fn manual_detector_runs_without_writer_lock_and_snapshot_drift_rolls_back_batch() -> Result<()> {
    let db_path = crate::db::test_support::unique_temp_db_path("manual-detector-lock");
    let seed_conn = Connection::open(&db_path)?;
    crate::migrate::run_migrations(&seed_conn)?;
    let first_id = crate::db::test_support::insert_legacy_pending_fixture(
        &seed_conn,
        crate::runtime_config::CODEX_HOST,
        "manual-first",
        "manual-project",
        "Bash",
        None,
        Some(r#"{"output":"first"}"#),
        None,
    )?;
    let second_id = crate::db::test_support::insert_legacy_pending_fixture(
        &seed_conn,
        crate::runtime_config::CODEX_HOST,
        "manual-second",
        "manual-project",
        "Bash",
        None,
        Some(r#"{"output":"second"}"#),
        Some("/tmp/remem-manual-detector"),
    )?;
    drop(seed_conn);

    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let worker_path = db_path.clone();
    let handle = std::thread::spawn(move || {
        let result = (|| -> Result<Vec<LegacyPendingMigration>> {
            let mut conn = Connection::open(worker_path)?;
            conn.busy_timeout(Duration::from_secs(5))?;
            let mut detector = move |_cwd: &str| {
                entered_tx.send(()).expect("announce manual detector entry");
                resume_rx.recv().expect("resume manual detector");
                None
            };
            migrate_legacy_pending_with_detector(
                &mut conn,
                Some("manual-project"),
                None,
                2,
                &mut detector,
            )
        })();
        done_tx
            .send(result)
            .expect("publish manual migration completion");
    });

    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("manual detector should run within timeout");
    let observer = Connection::open(&db_path)?;
    observer.busy_timeout(Duration::from_millis(100))?;
    let lock_probe = observer.execute_batch("BEGIN IMMEDIATE; ROLLBACK;");
    let changed_response = r#"{"output":"changed during manual preflight"}"#;
    let drift = observer.execute(
        "UPDATE pending_observations
         SET tool_response = ?2, updated_at_epoch = ?3
         WHERE id = ?1",
        params![second_id, changed_response, chrono::Utc::now().timestamp()],
    );
    resume_tx.send(()).expect("resume manual migration");
    let result = done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("manual migration should complete within timeout");
    handle.join().expect("manual migration thread should join");
    lock_probe?;
    drift?;

    let error = result.expect_err("manual snapshot drift must abort the batch");
    assert!(format!("{error:#}").contains("changed while preparing migration"));
    let states: Vec<(i64, String, String)> = observer
        .prepare(
            "SELECT id, status, tool_response
             FROM pending_observations
             WHERE id IN (?1, ?2)
             ORDER BY id",
        )?
        .query_map(params![first_id, second_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    assert_eq!(
        states,
        vec![
            (
                first_id,
                "pending".to_string(),
                r#"{"output":"first"}"#.to_string()
            ),
            (
                second_id,
                "pending".to_string(),
                changed_response.to_string()
            ),
        ]
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
    crate::db::test_support::cleanup_temp_db_files(&db_path);
    Ok(())
}

#[test]
fn manual_injected_detector_persists_precomputed_branch() -> Result<()> {
    let mut conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let project = "/tmp/remem-precomputed-branch-project";
    let cwd = "/tmp/remem-precomputed-branch-cwd";
    let pending_id = crate::db::test_support::insert_legacy_pending_fixture(
        &conn,
        crate::runtime_config::CODEX_HOST,
        "manual-precomputed-branch",
        project,
        "Bash",
        None,
        Some(r#"{"output":"precomputed"}"#),
        Some(cwd),
    )?;
    let mut detector_calls = 0;
    let mut detector = |detected_cwd: &str| {
        assert_eq!(detected_cwd, cwd);
        detector_calls += 1;
        Some("unique-injected-branch".to_string())
    };

    let migrated =
        migrate_legacy_pending_with_detector(&mut conn, Some(project), None, 1, &mut detector)?;

    assert_eq!(detector_calls, 1);
    assert_eq!(migrated.len(), 1);
    assert_eq!(migrated[0].pending_id, pending_id);
    let stored_branch: Option<String> = conn.query_row(
        "SELECT git_branch FROM workspaces WHERE root_path = ?1",
        [project],
        |row| row.get(0),
    )?;
    assert_eq!(stored_branch.as_deref(), Some("unique-injected-branch"));
    Ok(())
}
