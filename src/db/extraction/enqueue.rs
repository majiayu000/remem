use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::ExtractionTaskKind;

use super::ExtractionTask;

pub fn enqueue_followup_extraction_task(
    conn: &Connection,
    source: &ExtractionTask,
    task_kind: ExtractionTaskKind,
    high_watermark_event_id: i64,
) -> Result<i64> {
    let session_row_id = source
        .session_row_id
        .ok_or_else(|| anyhow::anyhow!("follow-up extraction task requires session_row_id"))?;
    let now = chrono::Utc::now().timestamp();
    let idempotency_key = if let Some(replay_range_id) = source.replay_range_id {
        format!(
            "{}:{}:{}:{}:replay:{}",
            source.host_id,
            source.project_id,
            session_row_id,
            task_kind.as_str(),
            replay_range_id
        )
    } else {
        format!(
            "{}:{}:{}:{}",
            source.host_id,
            source.project_id,
            session_row_id,
            task_kind.as_str()
        )
    };
    let cursor_event_id = source.replay_range_id.and(source.cursor_event_id);
    conn.execute(
        "INSERT INTO extraction_tasks
         (task_kind, host_id, workspace_id, project_id, session_row_id, priority, status,
          idempotency_key, cursor_event_id, high_watermark_event_id, attempts,
          next_retry_epoch, lease_owner, lease_expires_epoch, last_error, created_at_epoch,
          updated_at_epoch, replay_range_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?9, 0, NULL, NULL, NULL, NULL,
                 ?10, ?10, ?11)
         ON CONFLICT(idempotency_key) DO UPDATE SET
             high_watermark_event_id = MAX(COALESCE(extraction_tasks.high_watermark_event_id, 0), excluded.high_watermark_event_id),
             cursor_event_id = CASE
                 WHEN excluded.replay_range_id IS NOT NULL
                  AND extraction_tasks.status IN ('done', 'failed') THEN excluded.cursor_event_id
                 ELSE extraction_tasks.cursor_event_id
             END,
             status = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN 'pending'
                 ELSE extraction_tasks.status
             END,
             -- Reviving a terminal task resets its retry budget: the old
             -- attempts counted a range the exhaust path already skipped, so
             -- the new range must start with fresh attempts or it would fail
             -- terminally on its first defer.
             attempts = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN 0
                 ELSE extraction_tasks.attempts
             END,
             next_retry_epoch = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.next_retry_epoch
             END,
             last_error = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.last_error
             END,
             failure_class = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.failure_class
             END,
             failed_at_epoch = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.failed_at_epoch
             END,
             archived_at_epoch = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.archived_at_epoch
             END,
             replay_range_id = COALESCE(extraction_tasks.replay_range_id, excluded.replay_range_id),
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            task_kind.as_str(),
            source.host_id,
            source.workspace_id,
            source.project_id,
            session_row_id,
            task_kind.priority(),
            idempotency_key,
            cursor_event_id,
            high_watermark_event_id,
            now,
            source.replay_range_id
        ],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM extraction_tasks WHERE idempotency_key = ?1",
        params![idempotency_key],
        |row| row.get(0),
    )?)
}

pub fn enqueue_bounded_followup_extraction_task(
    conn: &Connection,
    source: &ExtractionTask,
    task_kind: ExtractionTaskKind,
    cursor_event_id: i64,
    high_watermark_event_id: i64,
) -> Result<i64> {
    if high_watermark_event_id <= cursor_event_id {
        bail!(
            "bounded follow-up extraction task requires high_watermark_event_id > cursor_event_id"
        );
    }
    let session_row_id = source
        .session_row_id
        .ok_or_else(|| anyhow::anyhow!("follow-up extraction task requires session_row_id"))?;
    let now = chrono::Utc::now().timestamp();
    let idempotency_key = if let Some(replay_range_id) = source.replay_range_id {
        format!(
            "{}:{}:{}:{}:bounded:{}:{}:replay:{}",
            source.host_id,
            source.project_id,
            session_row_id,
            task_kind.as_str(),
            cursor_event_id,
            high_watermark_event_id,
            replay_range_id
        )
    } else {
        format!(
            "{}:{}:{}:{}:bounded:{}:{}",
            source.host_id,
            source.project_id,
            session_row_id,
            task_kind.as_str(),
            cursor_event_id,
            high_watermark_event_id
        )
    };
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO extraction_tasks
         (task_kind, host_id, workspace_id, project_id, session_row_id, priority, status,
          idempotency_key, cursor_event_id, high_watermark_event_id, attempts,
          next_retry_epoch, lease_owner, lease_expires_epoch, last_error, created_at_epoch,
          updated_at_epoch, replay_range_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?9, 0, NULL, NULL, NULL, NULL,
                 ?10, ?10, ?11)
         ON CONFLICT(idempotency_key) DO UPDATE SET
             status = CASE
                 WHEN extraction_tasks.status = 'failed' THEN 'pending'
                 ELSE extraction_tasks.status
             END,
             attempts = CASE
                 WHEN extraction_tasks.status = 'failed' THEN 0
                 ELSE extraction_tasks.attempts
             END,
             cursor_event_id = CASE
                 WHEN extraction_tasks.status = 'failed' THEN excluded.cursor_event_id
                 ELSE extraction_tasks.cursor_event_id
             END,
             high_watermark_event_id = CASE
                 WHEN extraction_tasks.status = 'failed' THEN excluded.high_watermark_event_id
                 ELSE extraction_tasks.high_watermark_event_id
             END,
             next_retry_epoch = CASE
                 WHEN extraction_tasks.status = 'failed' THEN NULL
                 ELSE extraction_tasks.next_retry_epoch
             END,
             lease_owner = CASE
                 WHEN extraction_tasks.status = 'failed' THEN NULL
                 ELSE extraction_tasks.lease_owner
             END,
             lease_expires_epoch = CASE
                 WHEN extraction_tasks.status = 'failed' THEN NULL
                 ELSE extraction_tasks.lease_expires_epoch
             END,
             last_error = CASE
                 WHEN extraction_tasks.status = 'failed' THEN NULL
                 ELSE extraction_tasks.last_error
             END,
             failure_class = CASE
                 WHEN extraction_tasks.status = 'failed' THEN NULL
                 ELSE extraction_tasks.failure_class
             END,
             failed_at_epoch = CASE
                 WHEN extraction_tasks.status = 'failed' THEN NULL
                 ELSE extraction_tasks.failed_at_epoch
             END,
             archived_at_epoch = CASE
                 WHEN extraction_tasks.status = 'failed' THEN NULL
                 ELSE extraction_tasks.archived_at_epoch
             END,
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            task_kind.as_str(),
            source.host_id,
            source.workspace_id,
            source.project_id,
            session_row_id,
            task_kind.priority(),
            idempotency_key,
            cursor_event_id,
            high_watermark_event_id,
            now,
            source.replay_range_id
        ],
    )?;
    let task_id = tx.query_row(
        "SELECT id FROM extraction_tasks WHERE idempotency_key = ?1",
        params![idempotency_key],
        |row| row.get(0),
    )?;
    link_matching_replay_range_for_bounded_retry(
        &tx,
        task_id,
        task_kind,
        cursor_event_id,
        high_watermark_event_id,
        now,
    )?;
    tx.commit()?;
    Ok(task_id)
}

fn link_matching_replay_range_for_bounded_retry(
    conn: &Connection,
    task_id: i64,
    task_kind: ExtractionTaskKind,
    cursor_event_id: i64,
    high_watermark_event_id: i64,
    now: i64,
) -> Result<()> {
    let range_id = conn
        .query_row(
            "SELECT id
             FROM extraction_replay_ranges
             WHERE source_task_id = ?1
               AND task_kind = ?2
               AND from_event_id = ?3
               AND to_event_id = ?4
               AND status IN ('pending', 'failed', 'requeued')
             ORDER BY id DESC
             LIMIT 1",
            params![
                task_id,
                task_kind.as_str(),
                cursor_event_id + 1,
                high_watermark_event_id
            ],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(range_id) = range_id else {
        return Ok(());
    };
    let linked = conn.execute(
        "UPDATE extraction_tasks
         SET replay_range_id = ?1,
             updated_at_epoch = ?2
         WHERE id = ?3
           AND status = 'pending'
           AND cursor_event_id = ?4
           AND high_watermark_event_id = ?5",
        params![
            range_id,
            now,
            task_id,
            cursor_event_id,
            high_watermark_event_id
        ],
    )?;
    if linked != 1 {
        bail!("failed to link bounded extraction task {task_id} to replay range {range_id}");
    }
    conn.execute(
        "UPDATE extraction_replay_ranges
         SET status = 'requeued',
             replay_task_id = ?1,
             attempts = attempts + 1,
             updated_at_epoch = ?2
         WHERE id = ?3
           AND status IN ('pending', 'failed')",
        params![task_id, now, range_id],
    )?;
    Ok(())
}
