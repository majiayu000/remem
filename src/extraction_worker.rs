use anyhow::Result;
use tokio::time::Duration;

use crate::db;
use crate::memory_candidate::MemoryCandidateResult;

const DEPENDENCY_WAIT_RETRY_SECS: i64 = 300;

#[derive(Debug, PartialEq, Eq)]
enum ExtractionTaskOutcome {
    Deferred(String),
    // to_event_id is the highest event id actually covered by processing;
    // None means the full claim-time watermark range was covered.
    Done { to_event_id: Option<i64> },
    Waiting(String),
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
        Ok(Ok(ExtractionTaskOutcome::Done { to_event_id })) => {
            db::mark_extraction_task_done(
                &conn,
                task.id,
                lease_owner,
                to_event_id.or(task.high_watermark_event_id),
            )?;
            crate::log::info("worker", &format!("done extraction id={}", task.id));
        }
        Ok(Ok(ExtractionTaskOutcome::Deferred(msg))) => {
            let backoff = retry_backoff_secs(task.attempts);
            db::defer_claimed_extraction_task(&conn, &task, lease_owner, &msg, backoff)?;
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
        Ok(Ok(ExtractionTaskOutcome::Waiting(msg))) => {
            db::wait_extraction_task(
                &conn,
                task.id,
                lease_owner,
                &msg,
                DEPENDENCY_WAIT_RETRY_SECS,
            )?;
            crate::log::warn(
                "worker",
                &format!(
                    "extraction id={} waiting: {} (recheck in {}s)",
                    task.id,
                    crate::db::truncate_str(&msg, 300),
                    DEPENDENCY_WAIT_RETRY_SECS
                ),
            );
        }
        Ok(Err(e)) => {
            let msg = e.to_string();
            let backoff = retry_backoff_secs(task.attempts);
            db::mark_claimed_extraction_task_failed_or_retry(
                &conn,
                &task,
                lease_owner,
                &msg,
                backoff,
            )?;
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
            db::mark_claimed_extraction_task_failed_or_retry(
                &conn,
                &task,
                lease_owner,
                &msg,
                backoff,
            )?;
            crate::log::warn(
                "worker",
                &format!("extraction id={} timeout (retry in {}s)", task.id, backoff),
            );
        }
    }

    Ok(true)
}

pub(crate) async fn run_claimed_exact(
    mut task: db::ExtractionTask,
    profile: &str,
    lease_owner: &str,
    timeout_secs: u64,
) -> Result<()> {
    task.ai_profile = Some(profile.to_string());
    crate::log::info(
        "worker",
        &format!(
            "claimed exact extraction id={} range_id={} kind={} project={} profile={}",
            task.id,
            task.replay_range_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "<none>".to_string()),
            task.task_kind.as_str(),
            task.project,
            profile
        ),
    );

    let timed = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        process_extraction_task(&task),
    )
    .await;
    match timed {
        Ok(Ok(ExtractionTaskOutcome::Done { to_event_id })) => {
            let completed = to_event_id.or(task.high_watermark_event_id);
            if task
                .high_watermark_event_id
                .is_some_and(|high_watermark| completed != Some(high_watermark))
            {
                return archive_exact_outcome(
                    &task,
                    lease_owner,
                    "exact replay processed only part of the bounded event range",
                );
            }
            let conn = db::open_db()?;
            db::mark_extraction_task_done(&conn, task.id, lease_owner, completed)?;
            crate::log::info(
                "worker",
                &format!("done exact extraction id={} profile={profile}", task.id),
            );
            Ok(())
        }
        Ok(Ok(ExtractionTaskOutcome::Deferred(reason))) => archive_exact_outcome(
            &task,
            lease_owner,
            &format!("exact replay deferred: {reason}"),
        ),
        Ok(Ok(ExtractionTaskOutcome::Waiting(reason))) => archive_exact_outcome(
            &task,
            lease_owner,
            &format!("exact replay waiting: {reason}"),
        ),
        Ok(Err(error)) => {
            archive_exact_outcome(&task, lease_owner, &format!("exact replay failed: {error}"))
        }
        Err(_) => archive_exact_outcome(
            &task,
            lease_owner,
            &format!("exact replay timed out after {timeout_secs}s"),
        ),
    }
}

fn archive_exact_outcome(task: &db::ExtractionTask, lease_owner: &str, error: &str) -> Result<()> {
    let conn = db::open_db()?;
    db::archive_claimed_exact_replay_task(&conn, task.id, lease_owner, error)?;
    crate::log::error(
        "worker",
        &format!(
            "exact extraction archived id={} range_id={} error={}",
            task.id,
            task.replay_range_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "<none>".to_string()),
            crate::db::truncate_str(error, 300)
        ),
    );
    anyhow::bail!("{error}")
}

async fn process_extraction_task(task: &db::ExtractionTask) -> Result<ExtractionTaskOutcome> {
    match task.task_kind {
        db::ExtractionTaskKind::CapturedGitLink => {
            let mut conn = db::open_db()?;
            crate::captured_git::link_task_range(&mut conn, task)?;
            Ok(ExtractionTaskOutcome::Done { to_event_id: None })
        }
        db::ExtractionTaskKind::SessionRollup => {
            crate::session_rollup::process(task).await?;
            Ok(ExtractionTaskOutcome::Done { to_event_id: None })
        }
        db::ExtractionTaskKind::ObservationExtract => {
            crate::observation_extract::process(task).await?;
            Ok(ExtractionTaskOutcome::Done { to_event_id: None })
        }
        db::ExtractionTaskKind::MemoryCandidate => {
            let result = crate::memory_candidate::process(task).await?;
            Ok(memory_candidate_task_outcome(result))
        }
        db::ExtractionTaskKind::UserContextCandidate => {
            let result = crate::user_context::extraction::process(task).await?;
            Ok(user_context_candidate_task_outcome(result))
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
        MemoryCandidateResult::Written { to_event_id, .. } => ExtractionTaskOutcome::Done {
            to_event_id: Some(to_event_id),
        },
        _ => ExtractionTaskOutcome::Done { to_event_id: None },
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
        crate::graph_candidate::GraphCandidateResult::Waiting { reason } => {
            crate::log::warn(
                "worker",
                &format!(
                    "graph candidate extraction waiting for dependency: {}",
                    crate::db::truncate_str(&reason, 300)
                ),
            );
            ExtractionTaskOutcome::Waiting(reason)
        }
        _ => ExtractionTaskOutcome::Done { to_event_id: None },
    }
}

fn user_context_candidate_task_outcome(
    result: crate::user_context::extraction::UserContextCandidateExtractResult,
) -> ExtractionTaskOutcome {
    match result {
        crate::user_context::extraction::UserContextCandidateExtractResult::EmptyRange => {
            ExtractionTaskOutcome::Done { to_event_id: None }
        }
        crate::user_context::extraction::UserContextCandidateExtractResult::NoCandidates {
            to_event_id,
        } => ExtractionTaskOutcome::Done {
            to_event_id: Some(to_event_id),
        },
        crate::user_context::extraction::UserContextCandidateExtractResult::Written {
            to_event_id,
            ..
        } => ExtractionTaskOutcome::Done {
            to_event_id: Some(to_event_id),
        },
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
    fn done_outcome_carries_actual_covered_event_id_for_cursor_advance() {
        let outcome = memory_candidate_task_outcome(MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 0,
            to_event_id: 3,
        });

        assert_eq!(
            outcome,
            ExtractionTaskOutcome::Done {
                to_event_id: Some(3)
            },
            "done must report the event id actually covered by processing, not the claim-time watermark snapshot"
        );
    }

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

    #[test]
    fn graph_candidate_waiting_preserves_dependency_reason() {
        let outcome =
            graph_candidate_task_outcome(crate::graph_candidate::GraphCandidateResult::Waiting {
                reason: "memory review pending".to_string(),
            });

        assert_eq!(
            outcome,
            ExtractionTaskOutcome::Waiting("memory review pending".to_string())
        );
    }

    #[test]
    fn user_context_candidate_done_carries_covered_event_id() {
        let outcome = user_context_candidate_task_outcome(
            crate::user_context::extraction::UserContextCandidateExtractResult::Written {
                candidates: 1,
                promoted: 1,
                pending_review: 0,
                to_event_id: 42,
            },
        );

        assert_eq!(
            outcome,
            ExtractionTaskOutcome::Done {
                to_event_id: Some(42)
            }
        );
    }
}
