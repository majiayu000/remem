use anyhow::Result;
use tokio::time::{sleep, Duration, Instant};

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
const RULE_COMPILATION_SWEEP_INTERVAL_SECS: u64 = 60;

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

fn recover_expired_jobs(conn: &rusqlite::Connection) -> Result<()> {
    let batch = db::release_expired_job_leases(conn)?;
    for outcome in batch.outcomes {
        match outcome {
            db::ExpiredJobLeaseOutcome::Requeued {
                source_id,
                identity_kind,
            } => crate::log::warn(
                "worker",
                &format!(
                    "expired job recovery requeued source_id={source_id} identity={}",
                    identity_kind.as_str()
                ),
            ),
            db::ExpiredJobLeaseOutcome::Coalesced {
                source_id,
                canonical_id,
                identity_kind,
            } => crate::log::warn(
                "worker",
                &format!(
                    "expired job recovery coalesced source_id={source_id} canonical_id={canonical_id} identity={}",
                    identity_kind.as_str()
                ),
            ),
        }
    }
    Ok(())
}

fn mark_successful_job(conn: &rusqlite::Connection, job_id: i64, lease_owner: &str) -> Result<()> {
    if let Err(error) = db::mark_job_done(conn, job_id, lease_owner) {
        crate::log::error(
            "worker",
            &format!("job transition failed id={job_id} operation=done error={error}"),
        );
        return Err(error);
    }
    crate::log::info("worker", &format!("done id={job_id}"));
    Ok(())
}

fn record_failed_job_transition(
    conn: &rusqlite::Connection,
    job_id: i64,
    lease_owner: &str,
    error_message: &str,
    backoff_secs: i64,
) -> Result<()> {
    let transition = match db::mark_job_failed_or_retry(
        conn,
        job_id,
        lease_owner,
        error_message,
        backoff_secs,
    ) {
        Ok(transition) => transition,
        Err(error) => {
            crate::log::error(
                "worker",
                &format!("job transition failed id={job_id} operation=retry error={error}"),
            );
            return Err(error);
        }
    };
    match transition {
        db::JobTransitionOutcome::Transitioned => crate::log::warn(
            "worker",
            &format!(
                "job id={job_id} failed: {} (retry in {backoff_secs}s)",
                crate::db::truncate_str(error_message, 300)
            ),
        ),
        db::JobTransitionOutcome::Coalesced {
            source_id,
            canonical_id,
            identity_kind,
        } => crate::log::info(
            "worker",
            &format!(
                "job retry coalesced source_id={source_id} canonical_id={canonical_id} identity={}",
                identity_kind.as_str()
            ),
        ),
    }
    Ok(())
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

    let mut next_rule_compilation_sweep_at = Instant::now();
    loop {
        if Instant::now() >= next_rule_compilation_sweep_at {
            match job::run_rule_compilation_sweep().await {
                Ok(outcome) => {
                    if outcome.failures > 0 {
                        crate::log::error(
                            "rules",
                            &format!(
                                "rule compilation sweep completed with {}/{} project failure(s)",
                                outcome.failures, outcome.projects_seen
                            ),
                        );
                    }
                    if outcome.artifacts_changed > 0 {
                        crate::log::info(
                            "rules",
                            &format!(
                                "rule compilation sweep rebuilt {}/{} project artifact(s)",
                                outcome.artifacts_changed, outcome.projects_seen
                            ),
                        );
                    }
                }
                Err(error) => crate::log::error(
                    "rules",
                    &format!("rule compilation sweep skipped after setup failure: {error}"),
                ),
            }
            next_rule_compilation_sweep_at =
                Instant::now() + Duration::from_secs(RULE_COMPILATION_SWEEP_INTERVAL_SECS);
        }
        let mut conn = db::open_db()?;
        record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
        recover_expired_jobs(&conn)?;
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
                    mark_successful_job(&conn, job.id, &lease_owner)?;
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    let backoff = retry_backoff_secs(job.attempt_count);
                    record_failed_job_transition(&conn, job.id, &lease_owner, &msg, backoff)?;
                }
                Err(_) => {
                    let msg = format!("job timed out after {}s", JOB_TIMEOUT_SECS);
                    let backoff = retry_backoff_secs(job.attempt_count);
                    record_failed_job_transition(&conn, job.id, &lease_owner, &msg, backoff)?;
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
