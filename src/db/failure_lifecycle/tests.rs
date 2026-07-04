use super::*;
use crate::db::{CaptureEventInput, ExtractionTaskKind, JobType};
use anyhow::Result;
use rusqlite::{params, Connection};

#[test]
fn classifier_maps_known_permanent_patterns() {
    for error in [
        "schema mismatch in model output",
        "malformed payload",
        "unsupported version marker",
        "missing evidence rows",
        "rule candidate extraction is not implemented",
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
    let id = crate::db::enqueue_pending(
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
    let id = crate::db::enqueue_job(
        conn,
        "codex-cli",
        JobType::Summary,
        "/tmp/remem",
        Some("sess-failure"),
        "{}",
        100,
    )?;
    conn.execute(
        "UPDATE jobs
         SET state = 'failed',
             attempt_count = ?1,
             failure_class = ?2,
             failed_at_epoch = ?3,
             updated_at_epoch = ?3,
             next_retry_epoch = ?3
         WHERE id = ?4",
        params![attempts, class, failed_at, id],
    )?;
    Ok(id)
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
