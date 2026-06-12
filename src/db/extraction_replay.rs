use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::db::ExtractionTaskKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtractionReplayRange {
    pub id: i64,
    pub source_task_id: i64,
    pub replay_task_id: Option<i64>,
    pub task_kind: String,
    pub project: String,
    pub session_id: Option<String>,
    pub from_event_id: i64,
    pub to_event_id: i64,
    pub status: String,
    pub attempts: i64,
    pub updated_at_epoch: i64,
    pub last_error: Option<String>,
}

pub fn list_extraction_replay_ranges(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<ExtractionReplayRange>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.source_task_id, r.replay_task_id, r.task_kind, p.project_path,
                s.session_id, r.from_event_id, r.to_event_id, r.status, r.attempts,
                r.updated_at_epoch, r.last_error
         FROM extraction_replay_ranges r
         JOIN projects p ON p.id = r.project_id
         LEFT JOIN sessions s ON s.id = r.session_row_id
         WHERE r.status IN ('pending', 'failed', 'requeued', 'quarantined')
           AND (?1 IS NULL OR p.project_path = ?1)
         ORDER BY r.updated_at_epoch DESC, r.id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project, limit.max(1)], |row| {
        Ok(ExtractionReplayRange {
            id: row.get(0)?,
            source_task_id: row.get(1)?,
            replay_task_id: row.get(2)?,
            task_kind: row.get(3)?,
            project: row.get(4)?,
            session_id: row.get(5)?,
            from_event_id: row.get(6)?,
            to_event_id: row.get(7)?,
            status: row.get(8)?,
            attempts: row.get(9)?,
            updated_at_epoch: row.get(10)?,
            last_error: row.get(11)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

pub fn count_retryable_extraction_replay_ranges(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*)
         FROM (
             SELECT r.id
             FROM extraction_replay_ranges r
             JOIN projects p ON p.id = r.project_id
             WHERE r.status IN ('pending', 'failed')
               AND (?1 IS NULL OR p.project_path = ?1)
             ORDER BY r.updated_at_epoch ASC, r.id ASC
             LIMIT ?2
         )",
        params![project, limit.max(1)],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn query_retryable_replay_range_ids(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT r.id
         FROM extraction_replay_ranges r
         JOIN projects p ON p.id = r.project_id
         WHERE r.status IN ('pending', 'failed')
           AND (?1 IS NULL OR p.project_path = ?1)
         ORDER BY r.updated_at_epoch ASC, r.id ASC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project, limit.max(1)], |row| row.get::<_, i64>(0))?;
    crate::db::query::collect_rows(rows)
}

pub fn retry_extraction_replay_ranges(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let range_ids = query_retryable_replay_range_ids(&tx, project, limit)?;
    for range_id in &range_ids {
        enqueue_replay_extraction_task(&tx, *range_id)?;
    }
    tx.commit()?;
    Ok(range_ids.len())
}

pub fn quarantine_extraction_replay_ranges(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let range_ids = query_retryable_replay_range_ids(&tx, project, limit)?;
    let now = chrono::Utc::now().timestamp();
    for range_id in &range_ids {
        tx.execute(
            "UPDATE extraction_replay_ranges
             SET status = 'quarantined', updated_at_epoch = ?1
             WHERE id = ?2",
            params![now, range_id],
        )?;
    }
    tx.commit()?;
    Ok(range_ids.len())
}

fn enqueue_replay_extraction_task(conn: &Connection, range_id: i64) -> Result<i64> {
    let (task_kind, host_id, workspace_id, project_id, session_row_id, from_event_id, to_event_id) =
        conn.query_row(
            "SELECT task_kind, host_id, workspace_id, project_id, session_row_id,
                    from_event_id, to_event_id
             FROM extraction_replay_ranges
             WHERE id = ?1 AND status IN ('pending', 'failed')",
            params![range_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )?;
    let session_row_id = session_row_id.ok_or_else(|| {
        anyhow::anyhow!("extraction replay range {range_id} is missing session_row_id")
    })?;
    let task_kind_value = ExtractionTaskKind::from_db(&task_kind)?;
    let now = chrono::Utc::now().timestamp();
    let idempotency_key =
        format!("{host_id}:{project_id}:{session_row_id}:{task_kind}:replay-range:{range_id}");
    conn.execute(
        "INSERT INTO extraction_tasks
         (task_kind, host_id, workspace_id, project_id, session_row_id, priority, status,
          idempotency_key, cursor_event_id, high_watermark_event_id, attempts,
          next_retry_epoch, lease_owner, lease_expires_epoch, last_error, created_at_epoch,
          updated_at_epoch, replay_range_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?9, 0, NULL, NULL, NULL, NULL,
                 ?10, ?10, ?11)
         ON CONFLICT(idempotency_key) DO UPDATE SET
             status = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN 'pending'
                 ELSE extraction_tasks.status
             END,
             cursor_event_id = excluded.cursor_event_id,
             high_watermark_event_id = excluded.high_watermark_event_id,
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
             replay_range_id = excluded.replay_range_id,
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            task_kind,
            host_id,
            workspace_id,
            project_id,
            session_row_id,
            task_kind_value.priority(),
            idempotency_key,
            from_event_id - 1,
            to_event_id,
            now,
            range_id
        ],
    )?;
    let replay_task_id: i64 = conn.query_row(
        "SELECT id FROM extraction_tasks WHERE idempotency_key = ?1",
        params![idempotency_key],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE extraction_replay_ranges
         SET status = 'requeued',
             replay_task_id = ?1,
             attempts = attempts + 1,
             updated_at_epoch = ?2
         WHERE id = ?3",
        params![replay_task_id, now, range_id],
    )?;
    Ok(replay_task_id)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_exhausted_replay_range(
    conn: &Connection,
    source_task_id: i64,
    task_kind: &str,
    host_id: i64,
    workspace_id: i64,
    project_id: i64,
    session_row_id: Option<i64>,
    from_event_id: i64,
    to_event_id: i64,
    attempts: i64,
    err: &str,
    now: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO extraction_replay_ranges
         (source_task_id, task_kind, host_id, workspace_id, project_id, session_row_id,
          from_event_id, to_event_id, status, attempts, last_error, created_at_epoch,
          updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9, ?10, ?11, ?11)
         ON CONFLICT(source_task_id, from_event_id, to_event_id) DO UPDATE SET
             status = CASE
                 WHEN extraction_replay_ranges.status = 'quarantined' THEN 'quarantined'
                 ELSE 'pending'
             END,
             attempts = excluded.attempts,
             last_error = excluded.last_error,
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            source_task_id,
            task_kind,
            host_id,
            workspace_id,
            project_id,
            session_row_id,
            from_event_id,
            to_event_id,
            attempts,
            crate::db::truncate_str(err, 2000),
            now
        ],
    )?;
    conn.query_row(
        "SELECT id
         FROM extraction_replay_ranges
         WHERE source_task_id = ?1 AND from_event_id = ?2 AND to_event_id = ?3",
        params![source_task_id, from_event_id, to_event_id],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

pub(crate) fn mark_replay_range_replayed_if_done(
    conn: &Connection,
    task_id: i64,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE extraction_replay_ranges
         SET status = 'replayed',
             replay_task_id = COALESCE(replay_task_id, ?1),
             last_error = NULL,
             updated_at_epoch = ?2
         WHERE id = (
             SELECT replay_range_id FROM extraction_tasks
             WHERE id = ?1 AND status = 'done' AND replay_range_id IS NOT NULL
         )
           AND NOT EXISTS (
             SELECT 1 FROM extraction_tasks t
             WHERE t.replay_range_id = extraction_replay_ranges.id
               AND t.status != 'done'
         )",
        params![task_id, now],
    )?;
    Ok(())
}

pub(crate) fn mark_replay_range_failed(
    conn: &Connection,
    task_id: i64,
    now: i64,
    err: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE extraction_replay_ranges
         SET status = 'failed',
             replay_task_id = COALESCE(replay_task_id, ?1),
             attempts = COALESCE((SELECT attempts FROM extraction_tasks WHERE id = ?1), attempts),
             last_error = ?2,
             updated_at_epoch = ?3
         WHERE id = (
             SELECT replay_range_id FROM extraction_tasks
             WHERE id = ?1 AND replay_range_id IS NOT NULL
         )",
        params![task_id, crate::db::truncate_str(err, 2000), now],
    )?;
    Ok(())
}
