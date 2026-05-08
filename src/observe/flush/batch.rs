use anyhow::Result;

use crate::db;

use super::action::flush_action_batches;
use super::constants::{FLUSH_BATCH_SIZE, PENDING_LEASE_SECS};
use super::runtime::pending_retry_backoff_secs;
use super::task::flush_single_task;

pub async fn flush_pending(session_id: &str, project: &str) -> Result<usize> {
    let mut conn = db::open_db()?;
    let lease_owner = format!(
        "flush-{}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis(),
        crate::db::truncate_str(session_id, 8)
    );
    let pending = db::claim_pending(
        &conn,
        session_id,
        FLUSH_BATCH_SIZE,
        &lease_owner,
        PENDING_LEASE_SECS,
    )?;

    if pending.is_empty() {
        crate::log::info("flush", "no pending observations");
        return Ok(0);
    }

    let timer = crate::log::Timer::start(
        "flush",
        &format!("{} events project={}", pending.len(), project),
    );

    let (task_pending, action_pending): (Vec<_>, Vec<_>) = pending
        .iter()
        .enumerate()
        .partition::<Vec<_>, _>(|(_, pending)| pending.tool_name == "Task");
    let task_indices: Vec<usize> = task_pending.into_iter().map(|(index, _)| index).collect();
    let action_indices: Vec<usize> = action_pending.into_iter().map(|(index, _)| index).collect();

    let mut total_observations = 0usize;
    let mut titles = Vec::new();

    for &idx in &task_indices {
        let pending_item = &pending[idx];
        match flush_single_task(&mut conn, session_id, project, &lease_owner, pending_item).await {
            Ok(count) => {
                total_observations += count;
                if count > 0 {
                    crate::log::info(
                        "flush-task",
                        &format!("Task id={} → {} observations", pending_item.id, count),
                    );
                }
            }
            Err(err) => {
                let backoff = pending_retry_backoff_secs(pending_item.attempt_count);
                let err_msg = format!("task flush failed: {}", err);
                crate::log::warn(
                    "flush-task",
                    &format!(
                        "Task id={} flush failed (retry in {}s): {}",
                        pending_item.id, backoff, err
                    ),
                );
                if let Err(retry_err) = db::retry_pending_claimed(
                    &conn,
                    &lease_owner,
                    &[pending_item.id],
                    &err_msg,
                    backoff,
                ) {
                    crate::log::warn(
                        "flush-task",
                        &format!("retry mark failed id={}: {}", pending_item.id, retry_err),
                    );
                }
            }
        }
    }

    if !action_indices.is_empty() {
        let outcome = match flush_action_batches(
            &mut conn,
            session_id,
            project,
            &lease_owner,
            &pending,
            &action_indices,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(err) => {
                timer.done(&format!("AI error: {}", err));
                return Err(err);
            }
        };
        total_observations += outcome.total_observations;
        titles.extend(outcome.titles);
        if outcome.split_retries > 0 {
            crate::log::info(
                "flush",
                &format!("action batch split_retries={}", outcome.split_retries),
            );
        }
    }

    if total_observations == 0 {
        timer.done("0 observations");
        return Ok(0);
    }

    timer.done(&format!(
        "{} events → {} observations [{}]",
        pending.len(),
        total_observations,
        titles.join(", "),
    ));

    Ok(total_observations)
}
