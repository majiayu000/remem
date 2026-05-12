use anyhow::Result;

use crate::db;

use super::action::flush_action_batches;
use super::constants::{
    FLUSH_BATCH_SIZE, FLUSH_DRAIN_MAX_BATCHES, FLUSH_DRAIN_MAX_SECS, PENDING_LEASE_SECS,
};
use super::runtime::pending_retry_backoff_secs;
use super::task::flush_single_task;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationDrainOutcome {
    Drained,
    NeedsFollowUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlushPendingOnceOutcome {
    NoPending,
    Advanced,
}

pub async fn flush_pending(
    host: &str,
    session_id: &str,
    project: &str,
) -> Result<ObservationDrainOutcome> {
    let started = std::time::Instant::now();
    let mut batches = 0usize;

    loop {
        if batches >= FLUSH_DRAIN_MAX_BATCHES
            || started.elapsed() >= std::time::Duration::from_secs(FLUSH_DRAIN_MAX_SECS)
        {
            return follow_up_if_needed(host, project, session_id);
        }

        match flush_pending_once(host, session_id, project).await? {
            FlushPendingOnceOutcome::NoPending => return Ok(ObservationDrainOutcome::Drained),
            FlushPendingOnceOutcome::Advanced => batches += 1,
        }
    }
}

async fn flush_pending_once(
    host: &str,
    session_id: &str,
    project: &str,
) -> Result<FlushPendingOnceOutcome> {
    let mut conn = db::open_db()?;
    let recovered = db::release_expired_pending_claims(&conn)?;
    if recovered > 0 {
        crate::log::warn(
            "flush",
            &format!("released {} expired pending claim(s)", recovered),
        );
    }
    let lease_owner = format!(
        "flush-{}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis(),
        crate::db::truncate_str(session_id, 8)
    );
    let pending = db::claim_pending(
        &conn,
        host,
        project,
        session_id,
        FLUSH_BATCH_SIZE,
        &lease_owner,
        PENDING_LEASE_SECS,
    )?;

    if pending.is_empty() {
        crate::log::info("flush", "no pending observations");
        return Ok(FlushPendingOnceOutcome::NoPending);
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
        return Ok(FlushPendingOnceOutcome::Advanced);
    }

    timer.done(&format!(
        "{} events → {} observations [{}]",
        pending.len(),
        total_observations,
        titles.join(", "),
    ));

    Ok(FlushPendingOnceOutcome::Advanced)
}

fn follow_up_if_needed(
    host: &str,
    project: &str,
    session_id: &str,
) -> Result<ObservationDrainOutcome> {
    let conn = db::open_db()?;
    if db::count_pending_for_identity(&conn, host, project, session_id)? > 0 {
        Ok(ObservationDrainOutcome::NeedsFollowUp)
    } else {
        Ok(ObservationDrainOutcome::Drained)
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use crate::db;
    use crate::db::test_support::ScopedTestDataDir;
    use crate::observe::flush::constants::FLUSH_BATCH_SIZE;

    use super::{
        flush_pending, flush_pending_once, FlushPendingOnceOutcome, ObservationDrainOutcome,
    };

    #[tokio::test]
    async fn flush_pending_once_reports_no_pending_explicitly() -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("flush-batch-no-pending");
        let _conn = db::open_db()?;

        let outcome = flush_pending_once("codex-cli", "sess-empty", "proj-empty").await?;

        assert_eq!(outcome, FlushPendingOnceOutcome::NoPending);
        Ok(())
    }

    #[tokio::test]
    async fn flush_pending_continues_after_zero_observation_claimed_batch() -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("flush-batch-zero-continues");
        let conn = db::open_db()?;
        for idx in 0..(FLUSH_BATCH_SIZE + 1) {
            db::enqueue_pending(
                &conn,
                "codex-cli",
                "sess-zero",
                "proj-zero",
                "Task",
                Some(&format!("task {idx}")),
                Some("short"),
                None,
            )?;
        }

        let outcome = flush_pending("codex-cli", "sess-zero", "proj-zero").await?;

        assert_eq!(outcome, ObservationDrainOutcome::Drained);
        let failed_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pending_observations
             WHERE session_id = ?1 AND project = ?2 AND status = 'failed'",
            params!["sess-zero", "proj-zero"],
            |row| row.get(0),
        )?;
        let pending_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pending_observations
             WHERE session_id = ?1 AND project = ?2 AND status = 'pending'",
            params!["sess-zero", "proj-zero"],
            |row| row.get(0),
        )?;

        assert_eq!(failed_count, (FLUSH_BATCH_SIZE + 1) as i64);
        assert_eq!(pending_count, 0);
        Ok(())
    }

    #[tokio::test]
    async fn flush_pending_returns_follow_up_when_drain_budget_leaves_pending_rows(
    ) -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("flush-batch-follow-up");
        let conn = db::open_db()?;
        let rows = FLUSH_BATCH_SIZE * (super::FLUSH_DRAIN_MAX_BATCHES + 1);
        for idx in 0..rows {
            db::enqueue_pending(
                &conn,
                "codex-cli",
                "sess-follow",
                "proj-follow",
                "Task",
                Some(&format!("task {idx}")),
                Some("short"),
                None,
            )?;
        }

        let outcome = flush_pending("codex-cli", "sess-follow", "proj-follow").await?;

        assert_eq!(outcome, ObservationDrainOutcome::NeedsFollowUp);
        let failed_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pending_observations
             WHERE session_id = ?1 AND project = ?2 AND status = 'failed'",
            params!["sess-follow", "proj-follow"],
            |row| row.get(0),
        )?;
        let pending_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pending_observations
             WHERE session_id = ?1 AND project = ?2 AND status = 'pending'",
            params!["sess-follow", "proj-follow"],
            |row| row.get(0),
        )?;

        assert_eq!(
            failed_count,
            (FLUSH_BATCH_SIZE * super::FLUSH_DRAIN_MAX_BATCHES) as i64
        );
        assert_eq!(pending_count, FLUSH_BATCH_SIZE as i64);
        Ok(())
    }
}
