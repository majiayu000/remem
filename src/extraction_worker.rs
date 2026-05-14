use anyhow::Result;
use tokio::time::Duration;

use crate::db;

enum ExtractionTaskOutcome {
    Deferred(String),
    Done,
}

pub(crate) async fn run_next(
    lease_owner: &str,
    lease_secs: i64,
    timeout_secs: u64,
) -> Result<bool> {
    let mut conn = db::open_db()?;
    let Some(task) = db::claim_next_extraction_task(&mut conn, lease_owner, lease_secs)? else {
        return Ok(false);
    };

    crate::log::info(
        "worker",
        &format!(
            "claimed extraction id={} kind={} project={} attempt={}/{}",
            task.id,
            task.task_kind.as_str(),
            task.project,
            task.attempts + 1,
            db::EXTRACTION_TASK_MAX_ATTEMPTS
        ),
    );

    let timed = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        process_extraction_task(&task),
    )
    .await;
    let conn = db::open_db()?;
    match timed {
        Ok(Ok(ExtractionTaskOutcome::Done)) => {
            db::mark_extraction_task_done(
                &conn,
                task.id,
                lease_owner,
                task.high_watermark_event_id,
            )?;
            crate::log::info("worker", &format!("done extraction id={}", task.id));
        }
        Ok(Ok(ExtractionTaskOutcome::Deferred(msg))) => {
            let backoff = retry_backoff_secs(task.attempts);
            db::defer_extraction_task(&conn, task.id, lease_owner, &msg, backoff)?;
            crate::log::warn(
                "worker",
                &format!(
                    "extraction id={} deferred: {} (retry in {}s)",
                    task.id,
                    crate::db::truncate_str(&msg, 300),
                    backoff
                ),
            );
        }
        Ok(Err(e)) => {
            let msg = e.to_string();
            let backoff = retry_backoff_secs(task.attempts);
            db::mark_extraction_task_failed_or_retry(&conn, task.id, lease_owner, &msg, backoff)?;
            crate::log::warn(
                "worker",
                &format!(
                    "extraction id={} failed: {} (retry in {}s)",
                    task.id,
                    crate::db::truncate_str(&msg, 300),
                    backoff
                ),
            );
        }
        Err(_) => {
            let msg = format!("extraction task timed out after {}s", timeout_secs);
            let backoff = retry_backoff_secs(task.attempts);
            db::mark_extraction_task_failed_or_retry(&conn, task.id, lease_owner, &msg, backoff)?;
            crate::log::warn(
                "worker",
                &format!("extraction id={} timeout (retry in {}s)", task.id, backoff),
            );
        }
    }

    Ok(true)
}

async fn process_extraction_task(task: &db::ExtractionTask) -> Result<ExtractionTaskOutcome> {
    match task.task_kind {
        db::ExtractionTaskKind::SessionRollup => {
            crate::session_rollup::process(task).await?;
            Ok(ExtractionTaskOutcome::Done)
        }
        db::ExtractionTaskKind::ObservationExtract => {
            crate::observation_extract::process(task).await?;
            Ok(ExtractionTaskOutcome::Done)
        }
        db::ExtractionTaskKind::MemoryCandidate => {
            crate::memory_candidate::process(task).await?;
            Ok(ExtractionTaskOutcome::Done)
        }
        _ => Ok(ExtractionTaskOutcome::Deferred(format!(
            "extraction task kind '{}' is not implemented",
            task.task_kind.as_str()
        ))),
    }
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
