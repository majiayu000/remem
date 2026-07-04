use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::extraction_replay::record_exhausted_replay_range;

use super::loaders::ensure_task_updated;
use super::ExtractionTask;

// Terminal exhaustion: advance the cursor past the stuck range so a later
// coalesce revival only sees new events instead of re-reading the same
// undeliverable range forever.
pub(super) fn exhaust_extraction_task(
    conn: &Connection,
    task: &ExtractionTask,
    lease_owner: &str,
    attempts: i64,
    err: &str,
    now: i64,
) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .context("begin extraction exhaustion transaction")?;
    let result = exhaust_extraction_task_locked(conn, task, lease_owner, attempts, err, now);
    let (session_id, skipped_through, replay_range) = match result {
        Ok(output) => {
            conn.execute_batch("COMMIT")
                .context("commit extraction exhaustion transaction")?;
            output
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute_batch("ROLLBACK") {
                crate::log::error(
                    "extraction",
                    &format!(
                        "rollback failed after extraction exhaustion error: {rollback_error}; \
                         original error: {error:#}"
                    ),
                );
                return Err(error.context(format!(
                    "extraction exhaustion rollback also failed: {rollback_error}"
                )));
            }
            return Err(error);
        }
    };
    crate::log::error(
        "extraction",
        &format!(
            "task {} exhausted after {} attempts; session={} cursor advanced to {} with replay_range={} so later events stay extractable: {}",
            task.id,
            attempts,
            session_id.as_deref().unwrap_or("<unknown>"),
            skipped_through,
            replay_range
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            err
        ),
    );
    Ok(())
}

fn exhaust_extraction_task_locked(
    conn: &Connection,
    task: &ExtractionTask,
    lease_owner: &str,
    attempts: i64,
    err: &str,
    now: i64,
) -> Result<(Option<String>, i64, Option<i64>)> {
    let current = conn
        .query_row(
            "SELECT t.task_kind, t.host_id, t.workspace_id, t.project_id, t.session_row_id,
                    t.high_watermark_event_id, t.replay_range_id, s.session_id
             FROM extraction_tasks t
             LEFT JOIN sessions s ON s.id = t.session_row_id
             WHERE t.id = ?1 AND t.lease_owner = ?2 AND t.status = 'processing'",
            params![task.id, lease_owner],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((
        task_kind,
        host_id,
        workspace_id,
        project_id,
        session_row_id,
        current_high_watermark_event_id,
        replay_range_id,
        session_id,
    )) = current
    else {
        bail!("extraction task {} is not leased by this worker", task.id);
    };
    let replay_range_id = task.replay_range_id.or(replay_range_id);

    let cursor = task.cursor_event_id.unwrap_or(0);
    let skipped_through = task.high_watermark_event_id.unwrap_or(cursor);
    let replay_range = if let Some(range_id) = replay_range_id {
        conn.execute(
            "UPDATE extraction_replay_ranges
             SET status = 'failed',
                 attempts = ?1,
                 last_error = ?2,
                 failure_class = ?3,
                 failed_at_epoch = COALESCE(failed_at_epoch, ?4),
                 archived_at_epoch = NULL,
                 updated_at_epoch = ?4
             WHERE id = ?5",
            params![
                attempts,
                crate::db::truncate_str(err, 2000),
                crate::db::classify_failure(err).as_str(),
                now,
                range_id
            ],
        )?;
        Some(range_id)
    } else if skipped_through > cursor {
        Some(record_exhausted_replay_range(
            conn,
            task.id,
            &task_kind,
            host_id,
            workspace_id,
            project_id,
            session_row_id,
            cursor + 1,
            skipped_through,
            attempts,
            err,
            now,
        )?)
    } else {
        None
    };

    let still_has_later_events = current_high_watermark_event_id
        .map(|high_watermark| high_watermark > skipped_through)
        .unwrap_or(false);
    let next_status = if still_has_later_events {
        "pending"
    } else {
        "failed"
    };
    let next_attempts = if still_has_later_events { 0 } else { attempts };
    let updated = conn.execute(
        "UPDATE extraction_tasks
             SET status = ?1,
                 attempts = ?2,
                 cursor_event_id = ?3,
                 lease_owner = NULL,
                 lease_expires_epoch = NULL,
                 next_retry_epoch = NULL,
                 last_error = ?4,
                 failure_class = CASE WHEN ?1 = 'failed' THEN ?5 ELSE NULL END,
                 failed_at_epoch = CASE WHEN ?1 = 'failed' THEN COALESCE(failed_at_epoch, ?6) ELSE NULL END,
                 archived_at_epoch = NULL,
                 updated_at_epoch = ?6
             WHERE id = ?7 AND lease_owner = ?8 AND status = 'processing'",
        params![
            next_status,
            next_attempts,
            skipped_through,
            crate::db::truncate_str(err, 2000),
            crate::db::classify_failure(err).as_str(),
            now,
            task.id,
            lease_owner
        ],
    )?;
    ensure_task_updated(updated, task.id)?;
    Ok((session_id, skipped_through, replay_range))
}
