use rusqlite::params;

use crate::db::{self, test_support::ScopedTestDataDir};

use super::{cleanup, record_failed_job_transition, run};

#[tokio::test]
async fn worker_once_converges_expired_memory_and_persists_cooldown() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-automatic-cleanup");
    let conn = db::open_db()?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memories
         (project, title, content, memory_type, created_at_epoch, updated_at_epoch,
          status, scope, expires_at_epoch)
         VALUES ('/repo', 'Expired service', 'localhost service is running',
                 'discovery', ?1, ?1, 'active', 'project', ?2)",
        params![now - 100, now - 1],
    )?;
    let memory_id = conn.last_insert_rowid();
    drop(conn);

    run(true, 10).await?;

    let conn = db::open_db()?;
    let (status, valid_to_epoch): (String, Option<i64>) = conn.query_row(
        "SELECT status, valid_to_epoch FROM memories WHERE id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(status, "stale");
    assert!(valid_to_epoch.is_some());
    let first_counts: (i64, i64) = conn.query_row(
        "SELECT
             (SELECT COUNT(*) FROM jobs WHERE job_type = 'cleanup'),
             (SELECT COUNT(*) FROM maintenance_runs
              WHERE trigger = 'automatic' AND outcome = 'success')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(first_counts, (1, 1));
    drop(conn);

    run(true, 10).await?;

    let conn = db::open_db()?;
    let second_counts: (i64, i64) = conn.query_row(
        "SELECT
             (SELECT COUNT(*) FROM jobs WHERE job_type = 'cleanup'),
             (SELECT COUNT(*) FROM maintenance_runs
              WHERE trigger = 'automatic' AND outcome = 'success')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(second_counts, first_counts);
    Ok(())
}

#[test]
fn cleanup_retry_persists_only_redacted_failure_text() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-cleanup-redacted-error");
    let mut conn = db::open_db()?;
    let now = chrono::Utc::now().timestamp();
    let db::CleanupEnqueueDecision::Enqueued(job_id) =
        db::maybe_enqueue_cleanup_job_at(&conn, now)?
    else {
        anyhow::bail!("Cleanup should enqueue");
    };
    let owner = "cleanup-error-worker";
    let job = db::claim_ready_cleanup_job(&mut conn, owner, 60)?
        .ok_or_else(|| anyhow::anyhow!("Cleanup should be claimable"))?;
    assert_eq!(job.id, job_id);
    let error = anyhow::anyhow!("api_key=short-secret").context("delete old events");
    let message = cleanup::safe_failure_message(&error);

    record_failed_job_transition(
        &conn,
        job.id,
        job.job_type,
        &job.project,
        owner,
        &message,
        60,
    )?;

    let stored: String = conn.query_row(
        "SELECT last_error FROM jobs WHERE id = ?1",
        [job.id],
        |row| row.get(0),
    )?;
    assert!(stored.contains("[REDACTED]"), "{stored}");
    assert!(!stored.contains("short-secret"), "{stored}");
    Ok(())
}
