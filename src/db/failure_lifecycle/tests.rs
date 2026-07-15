use super::maintenance::{
    recover_due_job_candidate, requeue_due_jobs, set_job_recovery_test_seam, JobRecoveryOutcome,
    JobRecoveryTestSeam,
};
use super::*;
use crate::db::{CaptureEventInput, ExtractionTaskKind, JobType};
use anyhow::Result;
use rusqlite::{params, types::Value, Connection};
use std::sync::{Arc, Barrier};

#[test]
fn classifier_maps_known_permanent_patterns() {
    for error in [
        "schema mismatch in model output",
        "malformed payload",
        "unsupported version marker",
        "missing evidence rows",
        "rule candidate extraction is not implemented",
        "legacy summary writer retired",
    ] {
        assert_eq!(classify_failure(error), FailureClass::Permanent);
    }
}

#[test]
fn classifier_defaults_unknown_to_transient() {
    assert_eq!(
        classify_failure("the model returned a strange error"),
        FailureClass::Transient
    );
}

#[test]
fn classifier_treats_sqlite_schema_locks_as_transient() {
    assert_eq!(
        classify_failure("database schema is locked"),
        FailureClass::Transient
    );
}

fn setup_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn seed_pending_failure(conn: &Connection, failed_at: i64, class: &str) -> Result<i64> {
    let id = crate::db::test_support::insert_legacy_pending_fixture(
        conn,
        "codex-cli",
        "sess-failure",
        "/tmp/remem",
        "Bash",
        Some("{}"),
        Some("{}"),
        Some("/tmp/remem"),
    )?;
    conn.execute(
        "UPDATE pending_observations
         SET status = 'failed',
             failure_class = ?1,
             failed_at_epoch = ?2,
             updated_at_epoch = ?2
         WHERE id = ?3",
        params![class, failed_at, id],
    )?;
    Ok(id)
}

fn seed_job_failure(conn: &Connection, failed_at: i64, class: &str, attempts: i64) -> Result<i64> {
    insert_failed_job(
        conn,
        "codex-cli",
        JobType::Compress,
        "/tmp/remem",
        Some("sess-failure"),
        Some("transient failure"),
        failed_at,
        class,
        attempts,
    )
}

#[allow(clippy::too_many_arguments)]
fn insert_failed_job(
    conn: &Connection,
    host: &str,
    job_type: JobType,
    project: &str,
    session_id: Option<&str>,
    last_error: Option<&str>,
    failed_at: i64,
    class: &str,
    attempts: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO jobs
         (host, job_type, project, session_id, payload_json, state, priority,
          attempt_count, max_attempts, next_retry_epoch, last_error, failure_class,
          failed_at_epoch, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, '{}', 'failed', 100, ?5, 6, ?6, ?7, ?8, ?6, ?6, ?6)",
        params![
            host,
            job_type.as_str(),
            project,
            session_id,
            attempts,
            failed_at,
            last_error,
            class
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn job_snapshot(conn: &Connection, job_id: i64) -> Result<Vec<Value>> {
    Ok(conn.query_row(
        "SELECT id, host, job_type, project, session_id, payload_json, state,
                priority, attempt_count, max_attempts, lease_owner,
                lease_expires_epoch, next_retry_epoch, last_error,
                created_at_epoch, updated_at_epoch, failure_class,
                failed_at_epoch, archived_at_epoch
         FROM jobs WHERE id = ?1",
        params![job_id],
        |row| (0..19).map(|column| row.get(column)).collect(),
    )?)
}

fn seed_extraction_task(conn: &Connection) -> Result<(i64, i64)> {
    let outcome = crate::db::record_captured_event(
        conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-extraction",
            project: "/tmp/remem",
            cwd: Some("/tmp/remem"),
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: r#"{"tool_name":"Bash"}"#,
            task_kind: Some(ExtractionTaskKind::ObservationExtract),
        },
    )?;
    let task_id = outcome
        .extraction_task_id
        .expect("capture should coalesce extraction task");
    Ok((task_id, outcome.event_row_id))
}

fn seed_replay_range_failure(
    conn: &Connection,
    status: &str,
    failed_at: i64,
    class: &str,
    attempts: i64,
) -> Result<(i64, i64)> {
    let (task_id, event_row_id) = seed_extraction_task(conn)?;
    let (task_kind, host_id, workspace_id, project_id, session_row_id): (
        String,
        i64,
        i64,
        i64,
        Option<i64>,
    ) = conn.query_row(
        "SELECT task_kind, host_id, workspace_id, project_id, session_row_id
         FROM extraction_tasks
         WHERE id = ?1",
        [task_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    conn.execute(
        "INSERT INTO extraction_replay_ranges
         (source_task_id, task_kind, host_id, workspace_id, project_id, session_row_id,
          from_event_id, to_event_id, status, attempts, last_error, failure_class,
          failed_at_epoch, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8, ?9,
                 'replay failed', ?10, ?11, ?11, ?11)",
        params![
            task_id,
            task_kind,
            host_id,
            workspace_id,
            project_id,
            session_row_id,
            event_row_id,
            status,
            attempts,
            class,
            failed_at
        ],
    )?;
    Ok((task_id, conn.last_insert_rowid()))
}

#[test]
fn archive_moves_old_failures_out_of_actionable_stats() -> Result<()> {
    let conn = setup_conn()?;
    let now = 2_000_000;
    let old = now - 20 * SECONDS_PER_DAY;
    seed_pending_failure(&conn, old, "permanent")?;

    let before = query_failure_lifecycle_stats(&conn, now)?;
    assert_eq!(before.pending_observation.actionable_total, 1);

    let archived = archive_eligible_failures(&conn, now, FAILURE_RETENTION_DAYS)?;
    assert_eq!(archived.pending_observations, 1);

    let after = query_failure_lifecycle_stats(&conn, now)?;
    assert_eq!(after.pending_observation.actionable_total, 0);
    assert_eq!(after.pending_observation.archived, 1);
    assert_eq!(after.pending_observation.historical_archived, 1);
    Ok(())
}

#[test]
fn archive_counts_quarantined_replay_ranges_in_history() -> Result<()> {
    let conn = setup_conn()?;
    let now = 2_000_000;
    let old = now - 20 * SECONDS_PER_DAY;
    seed_replay_range_failure(&conn, "quarantined", old, "permanent", 3)?;

    let archived = archive_eligible_failures(&conn, now, FAILURE_RETENTION_DAYS)?;

    assert_eq!(archived.extraction_replay_ranges, 1);
    let stats = query_failure_lifecycle_stats(&conn, now)?;
    assert_eq!(stats.extraction_replay_range.actionable_total, 0);
    assert_eq!(stats.extraction_replay_range.archived, 1);
    assert_eq!(stats.extraction_replay_range.historical_archived, 1);
    Ok(())
}

#[test]
fn maintenance_requeues_due_transient_job_failure() -> Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    let job_id = seed_job_failure(&conn, now - 1_000, "transient", 0)?;

    let result = maintain_failure_lifecycle(&conn)?;

    assert_eq!(result.retried_jobs, 1);
    let (state, failure_class, failed_at): (String, Option<String>, Option<i64>) = conn.query_row(
        "SELECT state, failure_class, failed_at_epoch FROM jobs WHERE id = ?1",
        [job_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(state, "pending");
    assert_eq!(failure_class, None);
    assert_eq!(failed_at, None);
    Ok(())
}

#[test]
fn upgrade_summary_rejections_are_not_actionable_but_worker_rejections_are() -> Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    for (error, session_id) in [
        (
            "legacy summary job rejected during GH684 summary retirement upgrade; SessionRollup owns session summary output",
            "sess-upgrade-rejected",
        ),
        (
            "legacy Summary jobs are retired; SessionRollup owns session summary output",
            "sess-worker-rejected",
        ),
    ] {
        conn.execute(
            "INSERT INTO jobs
             (host, job_type, project, session_id, payload_json, state, priority,
              attempt_count, max_attempts, next_retry_epoch, last_error, failure_class,
              failed_at_epoch, created_at_epoch, updated_at_epoch)
             VALUES ('codex-cli', 'summary', '/tmp/remem', ?1, '{}', 'failed', 100,
                     3, 6, 0, ?2, 'permanent', ?3, ?3, ?3)",
            params![session_id, error, now - 1_000],
        )?;
    }

    let stats = query_failure_lifecycle_stats(&conn, now)?;

    assert_eq!(stats.job.actionable_total, 1);
    assert_eq!(stats.job.permanent, 1);
    Ok(())
}

#[test]
fn failure_lifecycle_auto_recovery_excludes_legacy_summary_and_recovers_ordinary() -> Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    let summary = insert_failed_job(
        &conn,
        "codex-cli",
        JobType::Summary,
        "/summary",
        Some("legacy"),
        Some("legacy audit"),
        now - 1_000,
        "transient",
        0,
    )?;
    let ordinary = insert_failed_job(
        &conn,
        "codex-cli",
        JobType::Compress,
        "/ordinary",
        None,
        Some("retry me"),
        now - 1_000,
        "transient",
        0,
    )?;
    let summary_before = job_snapshot(&conn, summary)?;

    let recovered = maintain_failure_lifecycle(&conn)?;

    assert_eq!((recovered.retried_jobs, recovered.coalesced_jobs), (1, 0));
    assert_eq!(job_snapshot(&conn, summary)?, summary_before);
    let state: String = conn.query_row(
        "SELECT state FROM jobs WHERE id = ?1",
        params![ordinary],
        |row| row.get(0),
    )?;
    assert_eq!(state, "pending");
    Ok(())
}

#[test]
fn failure_lifecycle_per_row_guard_preserves_legacy_summary() -> Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    let summary = insert_failed_job(
        &conn,
        "codex-cli",
        JobType::Summary,
        "/summary-guard",
        None,
        Some("preserve exactly"),
        now - 1_000,
        "transient",
        0,
    )?;
    let before = job_snapshot(&conn, summary)?;

    let outcome = recover_due_job_candidate(&conn, summary, now)?;

    assert_eq!(
        outcome,
        Some(JobRecoveryOutcome::SkippedRetiredSummary { source_id: summary })
    );
    assert_eq!(job_snapshot(&conn, summary)?, before);
    Ok(())
}

fn seed_collision(
    conn: &Connection,
    job_type: JobType,
    project: &str,
    attempt_count: i64,
    last_error: Option<&str>,
) -> Result<(i64, i64)> {
    let now = chrono::Utc::now().timestamp();
    let source = insert_failed_job(
        conn,
        "codex-cli",
        job_type,
        project,
        None,
        last_error,
        now - 1_000,
        "transient",
        attempt_count,
    )?;
    let canonical = crate::db::enqueue_job(conn, "codex-cli", job_type, project, None, "{}", 50)?;
    Ok((source, canonical))
}

#[test]
fn failure_lifecycle_auto_recovery_coalesces_mixed_active_identities_per_row() -> Result<()> {
    let conn = setup_conn()?;
    let pairs = [
        seed_collision(
            &conn,
            JobType::Compress,
            "/ordinary-mixed",
            0,
            Some("ordinary"),
        )?,
        seed_collision(&conn, JobType::Dream, "/dream-mixed", 0, Some("dream"))?,
        seed_collision(
            &conn,
            JobType::CompileRules,
            "/compile-mixed",
            0,
            Some("compile"),
        )?,
    ];

    let recovered = requeue_due_jobs(&conn, chrono::Utc::now().timestamp())?;

    assert_eq!((recovered.requeued, recovered.coalesced), (0, 3));
    for (source, canonical) in pairs {
        let (state, class, retry): (String, Option<String>, i64) = conn.query_row(
            "SELECT state, failure_class, next_retry_epoch FROM jobs WHERE id = ?1",
            params![source],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(
            (state.as_str(), class.as_deref(), retry),
            ("failed", Some("permanent"), 0)
        );
        assert!(recovered.outcomes.iter().any(|outcome| matches!(
            outcome,
            JobRecoveryOutcome::Coalesced { source_id, canonical_id, .. }
                if *source_id == source && *canonical_id == canonical
        )));
    }
    Ok(())
}

#[test]
fn failure_lifecycle_auto_recovery_preserves_source_error_and_does_not_repeat() -> Result<()> {
    let conn = setup_conn()?;
    let original = "x".repeat(2_500);
    let (source, canonical) = seed_collision(
        &conn,
        JobType::Compress,
        "/bounded-error",
        0,
        Some(&original),
    )?;
    let first = maintain_failure_lifecycle(&conn)?;
    assert_eq!(first.coalesced_jobs, 1);
    let error: String = conn.query_row(
        "SELECT last_error FROM jobs WHERE id = ?1",
        params![source],
        |row| row.get(0),
    )?;
    let marker = format!("[job_recovery_coalesced canonical_id={canonical} identity=ordinary]");
    assert!(error.starts_with('x'));
    assert!(error.ends_with(&marker));
    assert!(error.len() <= 2_000);
    let second = maintain_failure_lifecycle(&conn)?;
    assert_eq!((second.retried_jobs, second.coalesced_jobs), (0, 0));
    assert_eq!(
        conn.query_row(
            "SELECT last_error FROM jobs WHERE id = ?1",
            params![source],
            |row| row.get::<_, String>(0),
        )?,
        error
    );
    Ok(())
}

#[test]
fn failure_lifecycle_auto_recovery_preserves_source_attempt_count() -> Result<()> {
    let conn = setup_conn()?;
    let (source, _) = seed_collision(
        &conn,
        JobType::CompileRules,
        "/attempt-count",
        2,
        Some("real failure"),
    )?;
    requeue_due_jobs(&conn, chrono::Utc::now().timestamp())?;
    let attempts: i64 = conn.query_row(
        "SELECT attempt_count FROM jobs WHERE id = ?1",
        params![source],
        |row| row.get(0),
    )?;
    assert_eq!(attempts, 2);
    Ok(())
}

#[test]
fn failure_lifecycle_auto_recovery_null_or_empty_last_error_stores_canonical_marker() -> Result<()>
{
    let conn = setup_conn()?;
    for (project, error) in [("/null-error", None), ("/empty-error", Some(""))] {
        let (source, canonical) = seed_collision(&conn, JobType::Compress, project, 0, error)?;
        let outcome = recover_due_job_candidate(&conn, source, chrono::Utc::now().timestamp())?;
        assert!(matches!(
            outcome,
            Some(JobRecoveryOutcome::Coalesced { .. })
        ));
        let stored: String = conn.query_row(
            "SELECT last_error FROM jobs WHERE id = ?1",
            params![source],
            |row| row.get(0),
        )?;
        assert_eq!(
            stored,
            format!("[job_recovery_coalesced canonical_id={canonical} identity=ordinary]")
        );
    }
    Ok(())
}

#[test]
fn failure_lifecycle_job_recovery_two_wal_connections_coalesces_unique_race() -> Result<()> {
    let path = crate::db::test_support::unique_temp_db_path("failure-job-race");
    let conn = Connection::open(&path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    crate::migrate::run_migrations(&conn)?;
    let now = chrono::Utc::now().timestamp();
    let source = insert_failed_job(
        &conn,
        "codex-cli",
        JobType::Compress,
        "/wal-race",
        None,
        Some("retry"),
        now - 1_000,
        "transient",
        0,
    )?;
    let collected = Arc::new(Barrier::new(2));
    let committed = Arc::new(Barrier::new(2));
    set_job_recovery_test_seam(JobRecoveryTestSeam {
        candidates_collected: Some(Arc::clone(&collected)),
        canonical_committed: Some(Arc::clone(&committed)),
        skip_initial_lookup: true,
        unreadable_canonical_reread: false,
    });
    let thread_path = path.clone();
    let handle = std::thread::spawn(move || -> Result<i64> {
        let other = Connection::open(thread_path)?;
        other.pragma_update(None, "journal_mode", "WAL")?;
        other.busy_timeout(std::time::Duration::from_secs(30))?;
        collected.wait();
        let canonical = crate::db::enqueue_job(
            &other,
            "codex-cli",
            JobType::Compress,
            "/wal-race",
            None,
            "{}",
            50,
        )?;
        committed.wait();
        Ok(canonical)
    });

    let recovered = requeue_due_jobs(&conn, now)?;
    set_job_recovery_test_seam(JobRecoveryTestSeam::default());
    let canonical = handle.join().expect("canonical thread should join")?;
    assert_eq!(
        recovered.outcomes,
        vec![JobRecoveryOutcome::Coalesced {
            source_id: source,
            canonical_id: canonical,
            identity_kind: crate::db::JobIdentityKind::Ordinary,
        }]
    );
    drop(conn);
    crate::db::test_support::cleanup_temp_db_files(&path);
    Ok(())
}

#[test]
fn failure_lifecycle_job_recovery_unreadable_canonical_rolls_back_source() -> Result<()> {
    let conn = setup_conn()?;
    let (source, _) = seed_collision(
        &conn,
        JobType::Compress,
        "/unreadable-canonical",
        1,
        Some("preserve source"),
    )?;
    let before = job_snapshot(&conn, source)?;
    set_job_recovery_test_seam(JobRecoveryTestSeam {
        skip_initial_lookup: true,
        unreadable_canonical_reread: true,
        ..JobRecoveryTestSeam::default()
    });
    let error = requeue_due_jobs(&conn, chrono::Utc::now().timestamp())
        .expect_err("unreadable canonical must fail closed");
    set_job_recovery_test_seam(JobRecoveryTestSeam::default());
    assert!(error.to_string().contains("injected unreadable"));
    assert_eq!(job_snapshot(&conn, source)?, before);
    Ok(())
}

#[test]
fn maintenance_requeues_due_no_range_extraction_task_failure() -> Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    let (task_id, _) = seed_extraction_task(&conn)?;
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'failed',
             attempts = 0,
             failure_class = 'transient',
             failed_at_epoch = ?1,
             updated_at_epoch = ?1
         WHERE id = ?2",
        params![now - 1_000, task_id],
    )?;

    let result = maintain_failure_lifecycle(&conn)?;

    assert_eq!(result.retried_extraction_tasks, 1);
    let (status, failure_class, failed_at): (String, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT status, failure_class, failed_at_epoch FROM extraction_tasks WHERE id = ?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    assert_eq!(status, "pending");
    assert_eq!(failure_class, None);
    assert_eq!(failed_at, None);
    Ok(())
}

#[test]
fn maintenance_requeues_due_transient_replay_range() -> Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    let (task_id, event_row_id) = seed_extraction_task(&conn)?;
    let (task_kind, host_id, workspace_id, project_id, session_row_id): (
        String,
        i64,
        i64,
        i64,
        Option<i64>,
    ) = conn.query_row(
        "SELECT task_kind, host_id, workspace_id, project_id, session_row_id
         FROM extraction_tasks
         WHERE id = ?1",
        [task_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    conn.execute(
        "INSERT INTO extraction_replay_ranges
         (source_task_id, task_kind, host_id, workspace_id, project_id, session_row_id,
          from_event_id, to_event_id, status, attempts, last_error, failure_class,
          failed_at_epoch, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 'pending', 0,
                 'transient timeout', 'transient', ?8, ?8, ?8)",
        params![
            task_id,
            task_kind,
            host_id,
            workspace_id,
            project_id,
            session_row_id,
            event_row_id,
            now - 1_000
        ],
    )?;
    let range_id = conn.last_insert_rowid();

    let result = maintain_failure_lifecycle(&conn)?;

    assert_eq!(result.retried_extraction_replay_ranges, 1);
    let (status, attempts, failure_class, failed_at): (String, i64, Option<String>, Option<i64>) =
        conn.query_row(
            "SELECT status, attempts, failure_class, failed_at_epoch
         FROM extraction_replay_ranges WHERE id = ?1",
            [range_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    assert_eq!(status, "requeued");
    assert_eq!(attempts, 1);
    assert_eq!(failure_class, None);
    assert_eq!(failed_at, None);
    let replay_tasks: i64 = conn.query_row(
        "SELECT COUNT(*) FROM extraction_tasks
         WHERE replay_range_id = ?1 AND status = 'pending'",
        [range_id],
        |row| row.get(0),
    )?;
    assert_eq!(replay_tasks, 1);
    Ok(())
}

#[test]
fn purge_archived_failures_deletes_only_explicit_old_archives() -> Result<()> {
    let conn = setup_conn()?;
    let now = 5_000_000;
    let old = now - 120 * SECONDS_PER_DAY;
    let pending_id = seed_pending_failure(&conn, old, "permanent")?;
    archive_eligible_failures(&conn, now - 100 * SECONDS_PER_DAY, FAILURE_RETENTION_DAYS)?;
    conn.execute(
        "UPDATE pending_observations SET archived_at_epoch = ?1 WHERE id = ?2",
        params![old, pending_id],
    )?;

    let plan = count_archived_failures_to_purge_at(&conn, now, 90)?;
    assert_eq!(plan.pending_observations, 1);

    let purged = purge_archived_failures_at(&conn, now, 90)?;
    assert_eq!(purged.pending_observations, 1);
    let remaining: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations WHERE id = ?1",
        [pending_id],
        |row| row.get(0),
    )?;
    assert_eq!(remaining, 0);
    let history = query_failure_lifecycle_stats(&conn, now)?;
    assert_eq!(history.pending_observation.historical_purged, 1);
    Ok(())
}

#[test]
fn purge_dry_run_counts_tasks_released_by_same_replay_range_purge() -> Result<()> {
    let conn = setup_conn()?;
    let now = 5_000_000;
    let old = now - 120 * SECONDS_PER_DAY;
    let (task_id, range_id) = seed_replay_range_failure(&conn, "failed", old, "permanent", 3)?;
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'failed',
             attempts = 3,
             failure_class = 'permanent',
             failed_at_epoch = ?1,
             archived_at_epoch = ?1,
             updated_at_epoch = ?1
         WHERE id = ?2",
        params![old, task_id],
    )?;
    conn.execute(
        "UPDATE extraction_replay_ranges
         SET archived_at_epoch = ?1,
             updated_at_epoch = ?1
         WHERE id = ?2",
        params![old, range_id],
    )?;

    let plan = count_archived_failures_to_purge_at(&conn, now, 90)?;

    assert_eq!(plan.extraction_replay_ranges, 1);
    assert_eq!(plan.extraction_tasks, 1);
    let purged = purge_archived_failures_at(&conn, now, 90)?;
    assert_eq!(purged.extraction_replay_ranges, 1);
    assert_eq!(purged.extraction_tasks, 1);
    Ok(())
}
