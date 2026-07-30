use super::*;
use crate::db::{test_support::ScopedTestDataDir, CaptureEventInput, ExtractionTaskKind};
use rusqlite::params;

fn setup_conn() -> anyhow::Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn seed_extraction_task(conn: &rusqlite::Connection) -> anyhow::Result<i64> {
    let outcome = crate::db::record_captured_event(
        conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id: "legacy-bridge-backpressure",
            project: "/tmp/remem-legacy-bridge",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: r#"{"summary":"existing extraction work"}"#,
            task_kind: Some(ExtractionTaskKind::ObservationExtract),
        },
    )?;
    outcome
        .extraction_task_id
        .ok_or_else(|| anyhow::anyhow!("capture did not enqueue extraction"))
}

#[test]
fn once_schedule_allows_only_one_migration_attempt() {
    let started_at = Instant::now();
    let mut schedule = LegacyPendingMigrationSchedule::new(true, started_at);

    assert!(schedule.is_due(started_at));
    schedule.record_attempt(started_at);
    assert!(!schedule.is_due(started_at));
    assert!(
        !schedule.is_due(started_at + LEGACY_PENDING_MIGRATION_INTERVAL + Duration::from_secs(1))
    );
}

#[test]
fn daemon_schedule_waits_sixty_seconds_between_attempts() {
    let started_at = Instant::now();
    let mut schedule = LegacyPendingMigrationSchedule::new(false, started_at);

    assert!(schedule.is_due(started_at));
    schedule.record_attempt(started_at);
    assert!(
        !schedule.is_due(started_at + LEGACY_PENDING_MIGRATION_INTERVAL - Duration::from_millis(1))
    );
    assert!(schedule.is_due(started_at + LEGACY_PENDING_MIGRATION_INTERVAL));
}

#[test]
fn due_pending_and_processing_apply_backpressure_but_delayed_does_not() -> anyhow::Result<()> {
    let conn = setup_conn()?;
    let task_id = seed_extraction_task(&conn)?;
    let now = chrono::Utc::now().timestamp();

    assert!(!extraction_pipeline_is_idle(&conn)?);
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'pending', next_retry_epoch = ?1
         WHERE id = ?2",
        params![now + 3_600, task_id],
    )?;
    assert!(extraction_pipeline_is_idle(&conn)?);

    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'processing',
             lease_owner = 'existing-worker',
             lease_expires_epoch = ?1
         WHERE id = ?2",
        params![now + 300, task_id],
    )?;
    assert!(!extraction_pipeline_is_idle(&conn)?);

    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'done',
             lease_owner = NULL,
             lease_expires_epoch = NULL
         WHERE id = ?1",
        [task_id],
    )?;
    assert!(extraction_pipeline_is_idle(&conn)?);
    Ok(())
}

#[test]
fn active_extraction_work_does_not_consume_once_migration_slot() -> anyhow::Result<()> {
    let conn = setup_conn()?;
    let task_id = seed_extraction_task(&conn)?;
    let started_at = Instant::now();
    let mut schedule = LegacyPendingMigrationSchedule::new(true, started_at);

    assert!(!should_attempt_legacy_pending_migration(
        &conn, &schedule, started_at
    )?);
    assert!(schedule.is_due(started_at));

    conn.execute(
        "UPDATE extraction_tasks SET status = 'done' WHERE id = ?1",
        [task_id],
    )?;
    assert!(should_attempt_legacy_pending_migration(
        &conn, &schedule, started_at
    )?);
    schedule.record_attempt(started_at);
    assert!(!should_attempt_legacy_pending_migration(
        &conn, &schedule, started_at
    )?);
    Ok(())
}

#[test]
fn zero_progress_yield_does_not_consume_once_migration_slot() {
    let started_at = Instant::now();
    let mut schedule = LegacyPendingMigrationSchedule::new(true, started_at);
    let outcome = db::pending::admin::AutoLegacyMigrationOutcome {
        migrated: 0,
        yielded_to_current_work: true,
    };

    record_legacy_pending_migration_outcome(&mut schedule, started_at, &outcome);

    assert!(schedule.is_due(started_at));
}

#[test]
fn partial_progress_yield_consumes_once_migration_slot() {
    let started_at = Instant::now();
    let mut schedule = LegacyPendingMigrationSchedule::new(true, started_at);
    let outcome = db::pending::admin::AutoLegacyMigrationOutcome {
        migrated: 1,
        yielded_to_current_work: true,
    };

    record_legacy_pending_migration_outcome(&mut schedule, started_at, &outcome);

    assert!(!schedule.is_due(started_at));
}

#[test]
fn completed_zero_progress_attempt_consumes_once_migration_slot() {
    let started_at = Instant::now();
    let mut schedule = LegacyPendingMigrationSchedule::new(true, started_at);
    let outcome = db::pending::admin::AutoLegacyMigrationOutcome::default();

    record_legacy_pending_migration_outcome(&mut schedule, started_at, &outcome);

    assert!(!schedule.is_due(started_at));
}

#[test]
fn lifecycle_requeue_of_due_extraction_work_blocks_legacy_migration() -> anyhow::Result<()> {
    let conn = setup_conn()?;
    let task_id = seed_extraction_task(&conn)?;
    let now_epoch = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'failed',
             attempts = 0,
             next_retry_epoch = 0,
             failure_class = 'transient',
             failed_at_epoch = ?1,
             updated_at_epoch = ?1
         WHERE id = ?2",
        params![now_epoch - 1_000, task_id],
    )?;
    assert!(extraction_pipeline_is_idle(&conn)?);

    let maintenance = crate::db::maintain_failure_lifecycle(&conn)?;

    assert_eq!(maintenance.retried_extraction_tasks, 1);
    let schedule = LegacyPendingMigrationSchedule::new(true, Instant::now());
    assert!(!should_attempt_legacy_pending_migration(
        &conn,
        &schedule,
        Instant::now()
    )?);
    Ok(())
}

#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn worker_once_restart_redrives_failed_legacy_backlog() -> anyhow::Result<()> {
    let data_dir = ScopedTestDataDir::new("worker-legacy-restart");
    std::fs::create_dir_all(&data_dir.path)?;
    let stub_codex = data_dir.path.join("codex-stub.sh");
    super::tests::test_support::install_stub_codex(&stub_codex);
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value(
        "memory_ai.profiles.codex.path",
        stub_codex
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("stub path must be valid utf-8"))?,
    )?;
    let conn = crate::db::open_db()?;
    let pending_id = crate::db::test_support::insert_legacy_pending_fixture(
        &conn,
        crate::runtime_config::CODEX_HOST,
        "worker-died-before-retry",
        "/tmp/remem-legacy-restart",
        "Bash",
        Some(r#"{"cmd":"printf important"}"#),
        Some(r#"{"output":"important"}"#),
        Some("/tmp/remem-legacy-restart"),
    )?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE pending_observations
         SET status = 'failed', failure_class = 'transient',
             attempt_count = 3, next_retry_epoch = ?2,
             failed_at_epoch = ?3, archived_at_epoch = ?4,
             last_error = 'worker terminated during processing'
         WHERE id = ?1",
        params![pending_id, now - 1, now - 500, now - 400],
    )?;
    drop(conn);

    run(true, 10).await?;

    let conn = crate::db::open_db()?;
    let source: (String, i64, Option<i64>) = conn.query_row(
        "SELECT status, attempt_count, archived_at_epoch
         FROM pending_observations WHERE id = ?1",
        [pending_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(source, ("migrated".to_string(), 0, None));
    let failed_backlog: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations WHERE status = 'failed'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(failed_backlog, 0);
    let captured: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events
         WHERE event_id = ?1",
        [format!("legacy-pending-{pending_id}")],
        |row| row.get(0),
    )?;
    assert_eq!(captured, 1);
    let observations: i64 =
        conn.query_row("SELECT COUNT(*) FROM observations", [], |row| row.get(0))?;
    assert!(observations >= 1);
    Ok(())
}
