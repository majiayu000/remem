use anyhow::Result;
use tokio::time::{sleep, Duration};

use crate::db;

mod job;
mod lock;

// The lease is the maximum time another worker will wait before requeuing a
// job whose owner died, so `JOB_LEASE_SECS` must always exceed
// `JOB_TIMEOUT_SECS`. Otherwise a job that legitimately runs near the
// timeout could be claimed by a second worker before its current owner has
// given up, causing duplicate processing on hard kills. The grace window
// (60s) gives the active worker time to fail the timeout check and release.
const JOB_TIMEOUT_SECS: u64 = 420;
const JOB_LEASE_SECS: i64 = (JOB_TIMEOUT_SECS as i64) + 60;
const _: () = assert!(JOB_LEASE_SECS > JOB_TIMEOUT_SECS as i64);
const EXTRACTION_TASK_TIMEOUT_SECS: u64 = JOB_TIMEOUT_SECS;
const EMBEDDING_BACKFILL_IDLE_BATCH_SIZE: i64 = 128;

fn retry_backoff_secs(attempt: i64) -> i64 {
    match attempt {
        0 => 5,
        1 => 15,
        2 => 45,
        3 => 120,
        4 => 300,
        _ => 900,
    }
}

fn record_worker_heartbeat(
    conn: &rusqlite::Connection,
    lease_owner: &str,
    started_at_epoch: i64,
) -> Result<()> {
    db::upsert_worker_heartbeat(
        conn,
        lease_owner,
        i64::from(std::process::id()),
        started_at_epoch,
        chrono::Utc::now().timestamp(),
    )
}

fn run_idle_embedding_backfill(conn: &rusqlite::Connection) -> Result<bool> {
    match crate::retrieval::vector::reindex_memory_embeddings_with_report(
        conn,
        EMBEDDING_BACKFILL_IDLE_BATCH_SIZE,
    ) {
        Ok(report) if report.processed > 0 => {
            crate::log::info(
                "worker",
                &format!(
                    "backfilled {} memory embedding(s) for model={} dimensions={}",
                    report.processed, report.model, report.dimensions
                ),
            );
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(error)
            if crate::retrieval::embedding::is_local_embedding_model_unavailable_error(&error) =>
        {
            crate::log::error(
                "worker",
                &format!("memory embedding backfill deferred: {error}"),
            );
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

pub async fn run(once: bool, idle_sleep_ms: u64) -> Result<()> {
    let started_at_epoch = chrono::Utc::now().timestamp();
    let mode = if once { "once" } else { "daemon" };
    let lease_owner = db::current_worker_owner(
        mode,
        std::process::id(),
        chrono::Utc::now().timestamp_millis(),
    );
    let Some(_singleton) = lock::acquire_worker_singleton_for_mode(once)? else {
        crate::log::info("worker", "worker already running, exiting");
        return Ok(());
    };
    crate::log::info(
        "worker",
        &format!("start owner={} mode={}", lease_owner, mode),
    );
    {
        let conn = db::open_db()?;
        record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
    }

    loop {
        let mut conn = db::open_db()?;
        record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
        let recovered = db::requeue_stuck_jobs(&conn)?;
        if recovered > 0 {
            crate::log::warn("worker", &format!("requeued {} stuck job(s)", recovered));
        }
        let recovered_extraction = db::release_expired_extraction_task_leases(&conn)?;
        if recovered_extraction > 0 {
            crate::log::warn(
                "worker",
                &format!(
                    "released {} expired extraction task lease(s)",
                    recovered_extraction
                ),
            );
        }
        db::maintain_failure_lifecycle(&conn)?;
        if crate::extraction_worker::run_next(
            &lease_owner,
            JOB_LEASE_SECS,
            EXTRACTION_TASK_TIMEOUT_SECS,
        )
        .await?
        {
            continue;
        }

        if let Some(job) = db::claim_next_job(&mut conn, &lease_owner, JOB_LEASE_SECS)? {
            crate::log::info(
                "worker",
                &format!(
                    "claimed id={} type={} project={} attempt={}/{}",
                    job.id,
                    job.job_type.as_str(),
                    job.project,
                    job.attempt_count + 1,
                    job.max_attempts
                ),
            );

            let timed = tokio::time::timeout(
                Duration::from_secs(JOB_TIMEOUT_SECS),
                job::process_job(&job),
            )
            .await;
            let conn = db::open_db()?;
            match timed {
                Ok(Ok(())) => {
                    db::mark_job_done(&conn, job.id, &lease_owner)?;
                    crate::log::info("worker", &format!("done id={}", job.id));
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    let backoff = retry_backoff_secs(job.attempt_count);
                    db::mark_job_failed_or_retry(&conn, job.id, &lease_owner, &msg, backoff)?;
                    crate::log::warn(
                        "worker",
                        &format!(
                            "job id={} failed: {} (retry in {}s)",
                            job.id,
                            crate::db::truncate_str(&msg, 300),
                            backoff
                        ),
                    );
                }
                Err(_) => {
                    let msg = format!("job timed out after {}s", JOB_TIMEOUT_SECS);
                    let backoff = retry_backoff_secs(job.attempt_count);
                    db::mark_job_failed_or_retry(&conn, job.id, &lease_owner, &msg, backoff)?;
                    crate::log::warn(
                        "worker",
                        &format!("job id={} timeout (retry in {}s)", job.id, backoff),
                    );
                }
            }
            continue;
        }

        if run_idle_embedding_backfill(&conn)? {
            continue;
        }

        if once {
            break;
        }
        sleep(Duration::from_millis(idle_sleep_ms.max(100))).await;
        continue;
    }

    if !once {
        let conn = db::open_db()?;
        record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
    }
    crate::log::info("worker", "stopped");
    Ok(())
}

#[cfg(all(test, unix))]
mod tests;
