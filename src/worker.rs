use anyhow::Result;
use serde::Deserialize;
use tokio::time::{sleep, Duration};

use crate::{db, observe_flush, summarize};

const JOB_LEASE_SECS: i64 = 600;
const JOB_TIMEOUT_SECS: u64 = 420;

#[derive(Debug, Deserialize)]
struct ObservationPayload {
    session_id: String,
    project: String,
}

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

async fn process_job(job: &db::Job) -> Result<()> {
    match job.job_type {
        db::JobType::Observation => {
            let payload: ObservationPayload = serde_json::from_str(&job.payload_json)?;
            let _ = observe_flush::flush_pending(&payload.session_id, &payload.project).await?;
            Ok(())
        }
        db::JobType::Summary => summarize::process_summary_job_input(&job.payload_json).await,
        db::JobType::Compress => summarize::process_compress_job(&job.project).await,
    }
}

pub async fn run(once: bool, idle_sleep_ms: u64) -> Result<()> {
    let lease_owner = format!(
        "worker-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );
    crate::log::info("worker", &format!("start owner={}", lease_owner));

    loop {
        let mut conn = db::open_db()?;
        let recovered = db::requeue_stuck_jobs(&conn)?;
        if recovered > 0 {
            crate::log::warn("worker", &format!("requeued {} stuck job(s)", recovered));
        }

        let Some(job) = db::claim_next_job(&mut conn, &lease_owner, JOB_LEASE_SECS)? else {
            if once {
                break;
            }
            sleep(Duration::from_millis(idle_sleep_ms.max(100))).await;
            continue;
        };

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

        let timed =
            tokio::time::timeout(Duration::from_secs(JOB_TIMEOUT_SECS), process_job(&job)).await;
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
    }

    crate::log::info("worker", "stopped");
    Ok(())
}
