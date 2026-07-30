use anyhow::Result;
use rusqlite::{params, Connection};

use super::{
    execute_automatic_cleanup_job, execute_manual_cleanup, latest_automatic_cleanup_run,
    preview_cleanup, record_failure_after_rollback, CleanupPolicy, CleanupTrigger,
};
use crate::db::{self, test_support::ScopedTestDataDir};

fn runtime_db(label: &str) -> Result<(ScopedTestDataDir, Connection)> {
    let data_dir = ScopedTestDataDir::new(label);
    let conn = db::open_db()?;
    Ok((data_dir, conn))
}

fn insert_memory(
    conn: &Connection,
    title: &str,
    now_epoch: i64,
    expires_at_epoch: Option<i64>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO memories
         (project, title, content, memory_type, created_at_epoch, updated_at_epoch,
          status, scope, expires_at_epoch)
         VALUES ('/repo', ?1, 'content', 'discovery', ?2, ?2,
                 'active', 'project', ?3)",
        params![title, now_epoch - 100, expires_at_epoch],
    )?;
    Ok(conn.last_insert_rowid())
}

fn insert_processing_cleanup_job(
    conn: &Connection,
    now_epoch: i64,
    lease_owner: &str,
    payload_json: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO jobs
         (host, job_type, project, session_id, payload_json, state, priority,
          attempt_count, max_attempts, lease_owner, lease_expires_epoch,
          next_retry_epoch, created_at_epoch, updated_at_epoch)
         VALUES ('maintenance', 'cleanup', '__global__', NULL, ?1, 'processing',
                 1000, 0, 6, ?2, ?3, 0, ?4, ?4)",
        params![payload_json, lease_owner, now_epoch + 300, now_epoch],
    )?;
    Ok(conn.last_insert_rowid())
}

fn insert_archived_failed_job(conn: &Connection, now_epoch: i64) -> Result<i64> {
    conn.execute(
        "INSERT INTO jobs
         (host, job_type, project, session_id, payload_json, state, priority,
          attempt_count, max_attempts, next_retry_epoch, last_error,
          created_at_epoch, updated_at_epoch, failure_class, failed_at_epoch,
          archived_at_epoch)
         VALUES ('host', 'compress', '/repo', 'old', '{}', 'failed', 100,
                 6, 6, 0, 'old failure', ?1, ?1, 'permanent', ?1, ?1)",
        params![now_epoch - 120 * 86_400],
    )?;
    Ok(conn.last_insert_rowid())
}

#[test]
fn dry_run_plan_is_read_only() -> Result<()> {
    let (_data_dir, conn) = runtime_db("maintenance-dry-run")?;
    let now = chrono::Utc::now().timestamp();
    let memory_id = insert_memory(&conn, "expired", now, Some(now - 1))?;
    let plan = preview_cleanup(&conn, now, CleanupPolicy::manual(None)?)?;

    assert_eq!(plan.expired_memories_to_stale, 1);
    assert_eq!(
        conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            [memory_id],
            |row| row.get::<_, String>(0),
        )?,
        "active"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM maintenance_runs", [], |row| {
            row.get::<_, i64>(0)
        })?,
        0
    );
    Ok(())
}

#[test]
fn manual_plan_and_applied_counts_match_and_keep_current_memory() -> Result<()> {
    let (_data_dir, conn) = runtime_db("maintenance-plan-applied")?;
    let now = chrono::Utc::now().timestamp();
    let expired_id = insert_memory(&conn, "expired", now, Some(now - 1))?;
    let future_id = insert_memory(&conn, "future", now, Some(now + 600))?;
    let durable_id = insert_memory(&conn, "durable", now, None)?;
    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('/repo', 'inactive', 'active', ?1, ?1)",
        params![now - 20 * 86_400],
    )?;
    conn.execute(
        "INSERT INTO events
         (session_id, project, event_type, summary, created_at_epoch, retention_class)
         VALUES ('s', '/repo', 'file_edit', 'old edit', ?1, 'ephemeral')",
        params![now - 40 * 86_400],
    )?;

    let execution = execute_manual_cleanup(&conn, now, CleanupPolicy::manual(None)?)?;
    assert_eq!(execution.plan.expired_memories_to_stale, 1);
    assert_eq!(execution.applied.expired_memories_marked_stale, 1);
    assert_eq!(execution.plan.inactive_workstreams_to_pause, 1);
    assert_eq!(execution.applied.inactive_workstreams_paused, 1);
    assert_eq!(execution.plan.old_events_to_delete, 1);
    assert_eq!(execution.applied.old_events_deleted, 1);
    for (id, expected) in [
        (expired_id, "stale"),
        (future_id, "active"),
        (durable_id, "active"),
    ] {
        let status: String =
            conn.query_row("SELECT status FROM memories WHERE id = ?1", [id], |row| {
                row.get(0)
            })?;
        assert_eq!(status, expected);
    }
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM maintenance_runs
             WHERE \"trigger\" = 'manual' AND outcome = 'success'",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        1
    );
    Ok(())
}

#[test]
fn later_failure_rolls_back_all_effects_and_records_redacted_failure() -> Result<()> {
    let (_data_dir, conn) = runtime_db("maintenance-rollback")?;
    let now = chrono::Utc::now().timestamp();
    let memory_id = insert_memory(&conn, "expired", now, Some(now - 1))?;
    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('/repo', 'inactive', 'active', ?1, ?1)",
        params![now - 20 * 86_400],
    )?;
    let workstream_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO events
         (session_id, project, event_type, summary, created_at_epoch, retention_class)
         VALUES ('s', '/repo', 'file_edit', 'old edit', ?1, 'ephemeral')",
        params![now - 40 * 86_400],
    )?;
    conn.execute_batch(
        "CREATE TRIGGER fail_cleanup_event_delete
         BEFORE DELETE ON events
         BEGIN
           SELECT RAISE(ABORT, 'secret=super-secret cleanup tail failure');
         END;",
    )?;

    let error = execute_manual_cleanup(&conn, now, CleanupPolicy::manual(None)?)
        .expect_err("tail failure must roll back cleanup");
    assert!(error.to_string().contains("[REDACTED]"));
    assert!(!error.to_string().contains("super-secret"));
    let memory_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    let workstream_status: String = conn.query_row(
        "SELECT status FROM workstreams WHERE id = ?1",
        [workstream_id],
        |row| row.get(0),
    )?;
    assert_eq!(memory_status, "active");
    assert_eq!(workstream_status, "active");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM events", [], |row| row
            .get::<_, i64>(0))?,
        1
    );
    let (successes, failures, stored_error): (i64, i64, String) = conn.query_row(
        "SELECT
           SUM(outcome = 'success'),
           SUM(outcome = 'failure'),
           MAX(CASE WHEN outcome = 'failure' THEN error END)
         FROM maintenance_runs",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!((successes, failures), (0, 1));
    assert!(stored_error.contains("[REDACTED]"));
    assert!(!stored_error.contains("super-secret"));
    Ok(())
}

#[test]
fn contextual_inline_secret_is_redacted_from_return_and_ledger() -> Result<()> {
    let (_data_dir, conn) = runtime_db("maintenance-inline-secret")?;
    let now = chrono::Utc::now().timestamp();
    let error = anyhow::anyhow!("api_key=short-secret").context("delete old events");

    let returned =
        record_failure_after_rollback(&conn, CleanupTrigger::Manual, now, error).to_string();
    let stored: String = conn.query_row(
        "SELECT error FROM maintenance_runs WHERE outcome = 'failure'",
        [],
        |row| row.get(0),
    )?;

    for message in [returned, stored] {
        assert!(message.contains("[REDACTED]"), "{message}");
        assert!(!message.contains("short-secret"), "{message}");
    }
    Ok(())
}

#[test]
fn automatic_cleanup_ignores_payload_purge_and_completes_same_transaction() -> Result<()> {
    let (_data_dir, conn) = runtime_db("maintenance-automatic")?;
    let now = chrono::Utc::now().timestamp();
    let archived_job_id = insert_archived_failed_job(&conn, now)?;
    let cleanup_job_id =
        insert_processing_cleanup_job(&conn, now, "worker-a", r#"{"archived_failures":1}"#)?;

    let execution = execute_automatic_cleanup_job(&conn, cleanup_job_id, "worker-a", now)?;
    assert_eq!(
        execution.applied.archived_failures_purged,
        db::ArchivedFailurePurgePlan::default()
    );
    assert!(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE id = ?1)",
        [archived_job_id],
        |row| row.get::<_, bool>(0),
    )?);
    let state: String = conn.query_row(
        "SELECT state FROM jobs WHERE id = ?1",
        [cleanup_job_id],
        |row| row.get(0),
    )?;
    assert_eq!(state, "done");
    let latest = latest_automatic_cleanup_run(&conn, "success")?
        .expect("automatic success ledger must exist");
    assert_eq!(latest.job_id, Some(cleanup_job_id));
    assert!(latest.counts_json.is_some());
    assert!(latest.error.is_none());
    Ok(())
}

#[test]
fn cleanup_ledger_records_completion_time_after_start() -> Result<()> {
    let (_data_dir, conn) = runtime_db("maintenance-finished-at")?;
    let started_at_epoch = chrono::Utc::now().timestamp() - 5;
    let cleanup_job_id = insert_processing_cleanup_job(&conn, started_at_epoch, "worker-a", "{}")?;

    execute_automatic_cleanup_job(&conn, cleanup_job_id, "worker-a", started_at_epoch)?;
    let run = latest_automatic_cleanup_run(&conn, "success")?
        .expect("successful cleanup run should be recorded");

    assert_eq!(run.started_at_epoch, started_at_epoch);
    assert!(run.finished_at_epoch > run.started_at_epoch);
    Ok(())
}

#[test]
fn automatic_failure_keeps_job_claimed_for_worker_retry_transition() -> Result<()> {
    let (_data_dir, conn) = runtime_db("maintenance-automatic-failure")?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO events
         (session_id, project, event_type, summary, created_at_epoch, retention_class)
         VALUES ('s', '/repo', 'file_edit', 'old edit', ?1, 'ephemeral')",
        params![now - 40 * 86_400],
    )?;
    conn.execute_batch(
        "CREATE TRIGGER fail_automatic_event_delete
         BEFORE DELETE ON events
         BEGIN SELECT RAISE(ABORT, 'automatic cleanup injected failure'); END;",
    )?;
    let cleanup_job_id = insert_processing_cleanup_job(&conn, now, "worker-a", "{}")?;

    execute_automatic_cleanup_job(&conn, cleanup_job_id, "worker-a", now)
        .expect_err("injected failure must propagate");
    let (state, owner): (String, Option<String>) = conn.query_row(
        "SELECT state, lease_owner FROM jobs WHERE id = ?1",
        [cleanup_job_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(state, "processing");
    assert_eq!(owner.as_deref(), Some("worker-a"));
    let latest = latest_automatic_cleanup_run(&conn, "failure")?
        .expect("automatic failure ledger must exist");
    assert_eq!(latest.job_id, Some(cleanup_job_id));
    assert!(latest.counts_json.is_none());
    assert!(latest.error.is_some());
    Ok(())
}

#[test]
fn archived_failure_purge_requires_positive_explicit_manual_days() -> Result<()> {
    assert!(CleanupPolicy::manual(Some(0)).is_err());
    assert!(CleanupPolicy::manual(Some(-1)).is_err());

    let (_data_dir, conn) = runtime_db("maintenance-manual-purge")?;
    let now = chrono::Utc::now().timestamp();
    let archived_job_id = insert_archived_failed_job(&conn, now)?;
    let execution = execute_manual_cleanup(&conn, now, CleanupPolicy::manual(Some(30))?)?;

    assert_eq!(execution.plan.archived_failures_to_purge.jobs, 1);
    assert_eq!(execution.applied.archived_failures_purged.jobs, 1);
    assert!(!conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE id = ?1)",
        [archived_job_id],
        |row| row.get::<_, bool>(0),
    )?);
    Ok(())
}
