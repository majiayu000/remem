use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::{
    extraction_replay::{
        mark_replay_range_failed, mark_replay_range_replayed_if_done, record_exhausted_replay_range,
    },
    ExtractionTaskKind,
};

pub const EXTRACTION_TASK_MAX_ATTEMPTS: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionTask {
    pub id: i64,
    pub task_kind: ExtractionTaskKind,
    pub host_id: i64,
    pub workspace_id: i64,
    pub project_id: i64,
    pub session_row_id: Option<i64>,
    pub host: String,
    pub project: String,
    pub session_id: Option<String>,
    pub ai_profile: Option<String>,
    pub priority: i64,
    pub cursor_event_id: Option<i64>,
    pub high_watermark_event_id: Option<i64>,
    pub attempts: i64,
    pub replay_range_id: Option<i64>,
}

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

pub fn claim_next_extraction_task(
    conn: &mut Connection,
    lease_owner: &str,
    lease_secs: i64,
) -> Result<Option<ExtractionTask>> {
    let now = chrono::Utc::now().timestamp();
    let lease_expires = now + lease_secs.max(1);
    let tx = conn.transaction()?;
    let candidate: Option<i64> = tx
        .query_row(
            "SELECT id FROM extraction_tasks
             WHERE status = 'pending'
               AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?1)
             ORDER BY priority ASC, created_at_epoch ASC, id ASC
             LIMIT 1",
            params![now],
            |row| row.get(0),
        )
        .optional()?;

    let Some(task_id) = candidate else {
        tx.commit()?;
        return Ok(None);
    };

    let updated = tx.execute(
        "UPDATE extraction_tasks
         SET status = 'processing',
             lease_owner = ?1,
             lease_expires_epoch = ?2,
             updated_at_epoch = ?3
         WHERE id = ?4 AND status = 'pending'",
        params![lease_owner, lease_expires, now, task_id],
    )?;
    if updated == 0 {
        tx.commit()?;
        return Ok(None);
    }

    let task = load_claimed_extraction_task(&tx, task_id)?;
    tx.commit()?;
    Ok(Some(task))
}

pub fn release_expired_extraction_task_leases(conn: &Connection) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let count = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'pending',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             updated_at_epoch = ?1
         WHERE status = 'processing'
           AND lease_expires_epoch IS NOT NULL
           AND lease_expires_epoch < ?1",
        params![now],
    )?;
    Ok(count)
}

pub fn mark_extraction_task_done(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    completed_high_watermark_event_id: Option<i64>,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let updated = conn.execute(
        "UPDATE extraction_tasks
         SET status = CASE
                 WHEN ?4 IS NOT NULL
                  AND high_watermark_event_id IS NOT NULL
                  AND high_watermark_event_id > ?4 THEN 'pending'
                 ELSE 'done'
             END,
             cursor_event_id = ?4,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             next_retry_epoch = NULL,
             last_error = NULL,
             updated_at_epoch = ?1
         WHERE id = ?2 AND lease_owner = ?3 AND status = 'processing'",
        params![now, task_id, lease_owner, completed_high_watermark_event_id],
    )?;
    ensure_task_updated(updated, task_id)?;
    mark_replay_range_replayed_if_done(conn, task_id, now)
}

pub fn mark_extraction_task_failed(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    err: &str,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let updated = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'failed',
             attempts = attempts + 1,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             next_retry_epoch = NULL,
             last_error = ?1,
             updated_at_epoch = ?2
         WHERE id = ?3 AND lease_owner = ?4 AND status = 'processing'",
        params![
            crate::db::truncate_str(err, 2000),
            now,
            task_id,
            lease_owner
        ],
    )?;
    ensure_task_updated(updated, task_id)?;
    mark_replay_range_failed(conn, task_id, now, err)
}

pub fn defer_extraction_task(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    reason: &str,
    backoff_secs: i64,
) -> Result<()> {
    let task = load_claimed_extraction_task(conn, task_id)?;
    defer_claimed_extraction_task(conn, &task, lease_owner, reason, backoff_secs)
}

pub fn defer_claimed_extraction_task(
    conn: &Connection,
    task: &ExtractionTask,
    lease_owner: &str,
    reason: &str,
    backoff_secs: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let next_attempt = task.attempts + 1;
    if next_attempt >= EXTRACTION_TASK_MAX_ATTEMPTS {
        return exhaust_extraction_task(conn, task, lease_owner, next_attempt, reason, now);
    }

    let updated = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'pending',
             attempts = ?1,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             next_retry_epoch = ?2,
             last_error = ?3,
             updated_at_epoch = ?4
         WHERE id = ?5 AND lease_owner = ?6 AND status = 'processing'",
        params![
            next_attempt,
            now + backoff_secs.max(1),
            crate::db::truncate_str(reason, 2000),
            now,
            task.id,
            lease_owner
        ],
    )?;
    ensure_task_updated(updated, task.id)
}

pub fn wait_extraction_task(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    reason: &str,
    backoff_secs: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let updated = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'pending',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             next_retry_epoch = ?1,
             last_error = ?2,
             updated_at_epoch = ?3
         WHERE id = ?4 AND lease_owner = ?5 AND status = 'processing'",
        params![
            now + backoff_secs.max(1),
            crate::db::truncate_str(reason, 2000),
            now,
            task_id,
            lease_owner
        ],
    )?;
    ensure_task_updated(updated, task_id)
}

// Terminal exhaustion: advance the cursor past the stuck range so a later
// coalesce revival only sees new events instead of re-reading the same
// undeliverable range forever.
fn exhaust_extraction_task(
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
                 updated_at_epoch = ?3
             WHERE id = ?4",
            params![attempts, crate::db::truncate_str(err, 2000), now, range_id],
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
                 updated_at_epoch = ?5
             WHERE id = ?6 AND lease_owner = ?7 AND status = 'processing'",
        params![
            next_status,
            next_attempts,
            skipped_through,
            crate::db::truncate_str(err, 2000),
            now,
            task.id,
            lease_owner
        ],
    )?;
    ensure_task_updated(updated, task.id)?;
    Ok((session_id, skipped_through, replay_range))
}

pub fn mark_extraction_task_failed_or_retry(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    err: &str,
    backoff_secs: i64,
) -> Result<()> {
    let task = load_claimed_extraction_task(conn, task_id)?;
    mark_claimed_extraction_task_failed_or_retry(conn, &task, lease_owner, err, backoff_secs)
}

pub fn mark_claimed_extraction_task_failed_or_retry(
    conn: &Connection,
    task: &ExtractionTask,
    lease_owner: &str,
    err: &str,
    backoff_secs: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let next_attempt = task.attempts + 1;
    if next_attempt >= EXTRACTION_TASK_MAX_ATTEMPTS {
        return exhaust_extraction_task(conn, task, lease_owner, next_attempt, err, now);
    }

    let updated = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'pending',
             attempts = ?1,
             next_retry_epoch = ?2,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             last_error = ?3,
             updated_at_epoch = ?4
         WHERE id = ?5 AND lease_owner = ?6 AND status = 'processing'",
        params![
            next_attempt,
            now + backoff_secs.max(1),
            crate::db::truncate_str(err, 2000),
            now,
            task.id,
            lease_owner
        ],
    )?;
    ensure_task_updated(updated, task.id)
}

fn load_claimed_extraction_task(conn: &Connection, task_id: i64) -> Result<ExtractionTask> {
    let row = conn.query_row(
        "SELECT t.id, t.task_kind, t.host_id, t.workspace_id, t.project_id, t.session_row_id,
                h.name, p.project_path, s.session_id,
                t.priority, t.cursor_event_id, t.high_watermark_event_id, t.attempts,
                t.replay_range_id
         FROM extraction_tasks t
         JOIN hosts h ON h.id = t.host_id
         JOIN projects p ON p.id = t.project_id
         LEFT JOIN sessions s ON s.id = t.session_row_id
         WHERE t.id = ?1",
        params![task_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, Option<i64>>(13)?,
            ))
        },
    )?;

    let ai_profile = load_task_ai_profile(conn, row.2, row.4, row.5, row.11)?;
    Ok(ExtractionTask {
        id: row.0,
        task_kind: ExtractionTaskKind::from_db(&row.1)?,
        host_id: row.2,
        workspace_id: row.3,
        project_id: row.4,
        session_row_id: row.5,
        host: row.6,
        project: row.7,
        session_id: row.8,
        ai_profile,
        priority: row.9,
        cursor_event_id: row.10,
        high_watermark_event_id: row.11,
        attempts: row.12,
        replay_range_id: row.13,
    })
}

fn load_task_ai_profile(
    conn: &Connection,
    host_id: i64,
    project_id: i64,
    session_row_id: Option<i64>,
    high_watermark_event_id: Option<i64>,
) -> Result<Option<String>> {
    let Some(session_row_id) = session_row_id else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT COALESCE(
                    CASE
                        WHEN b.content_encoding = 'plain' THEN CAST(b.content_bytes AS TEXT)
                        ELSE NULL
                    END,
                    e.content_text,
                    ''
                ) AS content
         FROM captured_events e
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         WHERE e.host_id = ?1
           AND e.project_id = ?2
           AND e.session_row_id = ?3
           AND (?4 IS NULL OR e.id <= ?4)
         ORDER BY e.id DESC",
    )?;
    let contents = stmt
        .query_map(
            params![host_id, project_id, session_row_id, high_watermark_event_id],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(contents
        .iter()
        .find_map(|content| crate::runtime_config::profile_from_payload_text(content)))
}

fn ensure_task_updated(updated: usize, task_id: i64) -> Result<()> {
    if updated == 0 {
        bail!("extraction task {task_id} is not leased by this worker");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
