use anyhow::Result;
use tokio::time::Duration;

use crate::db;
use crate::memory_candidate::MemoryCandidateResult;

#[derive(Debug, PartialEq, Eq)]
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
            let result = crate::memory_candidate::process(task).await?;
            Ok(memory_candidate_task_outcome(result))
        }
        db::ExtractionTaskKind::GraphCandidate => {
            let result = crate::graph_candidate::process_graph_candidate_task(task).await?;
            Ok(graph_candidate_task_outcome(result))
        }
        _ => Ok(ExtractionTaskOutcome::Deferred(format!(
            "extraction task kind '{}' is not implemented",
            task.task_kind.as_str()
        ))),
    }
}

fn memory_candidate_task_outcome(result: MemoryCandidateResult) -> ExtractionTaskOutcome {
    match result {
        MemoryCandidateResult::Deferred { reason } => {
            crate::log::warn(
                "worker",
                &format!(
                    "memory candidate extraction deferred by model: {}",
                    crate::db::truncate_str(&reason, 300)
                ),
            );
            ExtractionTaskOutcome::Deferred(reason)
        }
        _ => ExtractionTaskOutcome::Done,
    }
}

fn graph_candidate_task_outcome(
    result: crate::graph_candidate::GraphCandidateResult,
) -> ExtractionTaskOutcome {
    match result {
        crate::graph_candidate::GraphCandidateResult::Deferred { reason } => {
            crate::log::warn(
                "worker",
                &format!(
                    "graph candidate extraction deferred by model: {}",
                    crate::db::truncate_str(&reason, 300)
                ),
            );
            ExtractionTaskOutcome::Deferred(reason)
        }
        _ => ExtractionTaskOutcome::Done,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_candidate_defer_preserves_range_for_reprocessing() {
        let outcome = memory_candidate_task_outcome(MemoryCandidateResult::Deferred {
            reason: "ambiguous conflict".to_string(),
        });

        assert_eq!(
            outcome,
            ExtractionTaskOutcome::Deferred("ambiguous conflict".to_string())
        );
    }

    #[test]
    fn graph_candidate_defer_preserves_range_for_reprocessing() {
        let outcome =
            graph_candidate_task_outcome(crate::graph_candidate::GraphCandidateResult::Deferred {
                reason: "ambiguous graph conflict".to_string(),
            });

        assert_eq!(
            outcome,
            ExtractionTaskOutcome::Deferred("ambiguous graph conflict".to_string())
        );
    }
}
