use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::extraction_replay::{mark_replay_range_failed, mark_replay_range_replayed_if_done};

use super::exhaust::exhaust_extraction_task;
use super::loaders::{ensure_task_updated, load_claimed_extraction_task};
use super::{ExtractionTask, EXTRACTION_TASK_MAX_ATTEMPTS};

pub fn claim_next_extraction_task(
    conn: &mut Connection,
    lease_owner: &str,
    lease_secs: i64,
) -> Result<Option<ExtractionTask>> {
    let now = chrono::Utc::now().timestamp();
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

    let task = claim_extraction_task_by_id_in_transaction(&tx, task_id, lease_owner, lease_secs)?;
    tx.commit()?;
    Ok(task)
}

#[cfg(test)]
pub(crate) fn claim_extraction_task_by_id(
    conn: &mut Connection,
    task_id: i64,
    lease_owner: &str,
    lease_secs: i64,
) -> Result<Option<ExtractionTask>> {
    let tx = conn.transaction()?;
    let task = claim_extraction_task_by_id_in_transaction(&tx, task_id, lease_owner, lease_secs)?;
    tx.commit()?;
    Ok(task)
}

pub(crate) fn claim_extraction_task_by_id_in_transaction(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    lease_secs: i64,
) -> Result<Option<ExtractionTask>> {
    let now = chrono::Utc::now().timestamp();
    let lease_expires = now + lease_secs.max(1);
    let updated = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'processing',
             lease_owner = ?1,
             lease_expires_epoch = ?2,
             updated_at_epoch = ?3
         WHERE id = ?4
           AND status = 'pending'
           AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?3)",
        params![lease_owner, lease_expires, now, task_id],
    )?;
    if updated == 0 {
        return Ok(None);
    }

    Ok(Some(load_claimed_extraction_task(conn, task_id)?))
}

pub(crate) fn claim_next_extraction_task_for_replay_range(
    conn: &mut Connection,
    range_id: i64,
    lease_owner: &str,
    lease_secs: i64,
) -> Result<Option<ExtractionTask>> {
    anyhow::ensure!(
        crate::db::is_exact_replay_worker_owner(lease_owner),
        "exact replay follow-up claim requires an exact replay worker owner"
    );
    let now = chrono::Utc::now().timestamp();
    let tx = conn.transaction()?;
    let candidate = tx
        .query_row(
            "SELECT id
             FROM extraction_tasks
             WHERE replay_range_id = ?1
               AND status = 'pending'
               AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?2)
             ORDER BY priority DESC, id ASC
             LIMIT 1",
            params![range_id, now],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(task_id) = candidate else {
        let status: String = tx.query_row(
            "SELECT status FROM extraction_replay_ranges WHERE id = ?1",
            params![range_id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            status == "replayed",
            "exact replay range {range_id} has no retry-ready follow-up and remains {status}"
        );
        tx.commit()?;
        return Ok(None);
    };
    let task = claim_extraction_task_by_id_in_transaction(&tx, task_id, lease_owner, lease_secs)?;
    let task = task.ok_or_else(|| {
        anyhow::anyhow!("exact replay follow-up task {task_id} could not be claimed")
    })?;
    tx.commit()?;
    Ok(Some(task))
}

pub fn release_expired_extraction_task_leases(conn: &Connection) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let tx = conn.unchecked_transaction()?;
    let expired = {
        let mut stmt = tx.prepare(
            "SELECT id, lease_owner
             FROM extraction_tasks
             WHERE status = 'processing'
               AND lease_expires_epoch IS NOT NULL
               AND lease_expires_epoch < ?1
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![now], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        crate::db::query::collect_rows(rows)?
    };

    for (task_id, lease_owner) in &expired {
        if let Some(exact_owner) = lease_owner
            .as_deref()
            .filter(|owner| crate::db::is_exact_replay_worker_owner(owner))
        {
            archive_claimed_exact_replay_task_in_transaction(
                &tx,
                *task_id,
                exact_owner,
                "exact replay worker lease expired; rerun the locked exact recovery command",
                now,
            )?;
        } else {
            let updated = tx.execute(
                "UPDATE extraction_tasks
                 SET status = 'pending',
                     lease_owner = NULL,
                     lease_expires_epoch = NULL,
                     updated_at_epoch = ?1
                 WHERE id = ?2
                   AND status = 'processing'
                   AND ((?3 IS NULL AND lease_owner IS NULL) OR lease_owner = ?3)",
                params![now, task_id, lease_owner],
            )?;
            ensure_task_updated(updated, *task_id)?;
        }
    }
    tx.commit()?;
    Ok(expired.len())
}

pub(crate) fn archive_claimed_exact_replay_task(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    error: &str,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    archive_claimed_exact_replay_task_in_transaction(
        &tx,
        task_id,
        lease_owner,
        error,
        chrono::Utc::now().timestamp(),
    )?;
    tx.commit()?;
    Ok(())
}

fn archive_claimed_exact_replay_task_in_transaction(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    error: &str,
    now: i64,
) -> Result<()> {
    anyhow::ensure!(
        crate::db::is_exact_replay_worker_owner(lease_owner),
        "exact replay archive requires an exact replay worker owner"
    );
    let replay_range_id: i64 = conn.query_row(
        "SELECT replay_range_id
         FROM extraction_tasks
         WHERE id = ?1 AND status = 'processing' AND lease_owner = ?2",
        params![task_id, lease_owner],
        |row| row.get(0),
    )?;
    let updated = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'failed',
             attempts = attempts + 1,
             next_retry_epoch = NULL,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             last_error = ?1,
             failure_class = ?2,
             failed_at_epoch = COALESCE(failed_at_epoch, ?3),
             archived_at_epoch = ?3,
             updated_at_epoch = ?3
         WHERE id = ?4 AND status = 'processing' AND lease_owner = ?5",
        params![
            crate::db::truncate_str(error, 2000),
            crate::db::classify_failure(error).as_str(),
            now,
            task_id,
            lease_owner
        ],
    )?;
    ensure_task_updated(updated, task_id)?;
    crate::db::extraction_replay::archive_exact_replay_range_after_task_failure(
        conn,
        replay_range_id,
        task_id,
        error,
        now,
    )
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
             failure_class = NULL,
             failed_at_epoch = NULL,
             archived_at_epoch = NULL,
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
             failure_class = ?2,
             failed_at_epoch = COALESCE(failed_at_epoch, ?3),
             archived_at_epoch = NULL,
             updated_at_epoch = ?3
         WHERE id = ?4 AND lease_owner = ?5 AND status = 'processing'",
        params![
            crate::db::truncate_str(err, 2000),
            crate::db::classify_failure(err).as_str(),
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
             failure_class = NULL,
             failed_at_epoch = NULL,
             archived_at_epoch = NULL,
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
             failure_class = NULL,
             failed_at_epoch = NULL,
             archived_at_epoch = NULL,
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
    if crate::db::classify_failure(err) == crate::db::FailureClass::Permanent
        || next_attempt >= EXTRACTION_TASK_MAX_ATTEMPTS
    {
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
             failure_class = NULL,
             failed_at_epoch = NULL,
             archived_at_epoch = NULL,
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
