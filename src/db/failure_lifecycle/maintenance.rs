use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::sql::{archived_replay_range_ids, id_placeholders, purgeable_extraction_task_ids};
use super::{FAILURE_RETRY_BASE_SECS, MAX_FAILURE_AUTO_RETRIES};

#[derive(Debug, Clone, Copy)]
pub(super) struct ArchiveSurface {
    pub(super) surface: &'static str,
    pub(super) table: &'static str,
    pub(super) failed_predicate: &'static str,
    pub(super) eligible_extra: &'static str,
}

pub(super) fn retry_due_extraction_replay_ranges(
    conn: &Connection,
    now_epoch: i64,
) -> Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.attempts
         FROM extraction_replay_ranges r
         WHERE r.status IN ('pending', 'failed')
           AND r.archived_at_epoch IS NULL
           AND COALESCE(r.failure_class, 'transient') = 'transient'
           AND r.attempts < ?1
           AND COALESCE(r.failed_at_epoch, r.updated_at_epoch, r.created_at_epoch) + (?2 * (1 << r.attempts)) <= ?3
           AND NOT EXISTS (
             SELECT 1 FROM extraction_tasks t
             WHERE t.replay_range_id = r.id
               AND t.status IN ('pending', 'processing')
           )
         ORDER BY COALESCE(r.failed_at_epoch, r.updated_at_epoch, r.created_at_epoch) ASC, r.id ASC
         LIMIT 25",
    )?;
    let rows = stmt.query_map(
        params![MAX_FAILURE_AUTO_RETRIES, FAILURE_RETRY_BASE_SECS, now_epoch],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    let mut count = 0;
    for row in rows {
        let (range_id, attempts) = row?;
        crate::log::info(
            "failure_lifecycle",
            &format!(
                "auto-retry extraction_replay_range id={} class=transient attempt={}/{}",
                range_id,
                attempts + 1,
                MAX_FAILURE_AUTO_RETRIES
            ),
        );
        crate::db::extraction_replay::enqueue_replay_extraction_task(conn, range_id)?;
        count += 1;
    }
    Ok(count)
}

pub(super) fn requeue_due_extraction_tasks(conn: &Connection, now_epoch: i64) -> Result<usize> {
    let changed = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'pending',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             next_retry_epoch = NULL,
             failure_class = NULL,
             failed_at_epoch = NULL,
             archived_at_epoch = NULL,
             updated_at_epoch = ?3
         WHERE id IN (
             SELECT id FROM extraction_tasks
             WHERE status = 'failed'
               AND replay_range_id IS NULL
               AND archived_at_epoch IS NULL
               AND COALESCE(failure_class, 'transient') = 'transient'
               AND attempts < ?1
               AND COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch) + (?2 * (1 << attempts)) <= ?3
             ORDER BY COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch) ASC, id ASC
             LIMIT 25
         )",
        params![MAX_FAILURE_AUTO_RETRIES, FAILURE_RETRY_BASE_SECS, now_epoch],
    )?;
    if changed > 0 {
        crate::log::info(
            "failure_lifecycle",
            &format!(
                "auto-requeued {} no-range extraction task failure(s)",
                changed
            ),
        );
    }
    Ok(changed)
}

pub(super) fn requeue_due_jobs(conn: &Connection, now_epoch: i64) -> Result<usize> {
    let changed = conn.execute(
        "UPDATE jobs
         SET state = 'pending',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             next_retry_epoch = ?3,
             failure_class = NULL,
             failed_at_epoch = NULL,
             archived_at_epoch = NULL,
             updated_at_epoch = ?3
         WHERE id IN (
             SELECT id FROM jobs
             WHERE state = 'failed'
               AND archived_at_epoch IS NULL
               AND COALESCE(failure_class, 'transient') = 'transient'
               AND attempt_count < ?1
               AND COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch) + (?2 * (1 << attempt_count)) <= ?3
             ORDER BY COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch) ASC, id ASC
             LIMIT 25
         )",
        params![MAX_FAILURE_AUTO_RETRIES, FAILURE_RETRY_BASE_SECS, now_epoch],
    )?;
    if changed > 0 {
        crate::log::info(
            "failure_lifecycle",
            &format!("auto-requeued {} failed job(s)", changed),
        );
    }
    Ok(changed)
}

pub(super) fn archive_surface(
    conn: &Connection,
    surface: ArchiveSurface,
    cutoff_epoch: i64,
    now_epoch: i64,
) -> Result<usize> {
    rollup_surface(
        conn,
        surface.surface,
        surface.table,
        surface.failed_predicate,
        &format!(
            "archived_at_epoch IS NULL
             AND COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch) < {cutoff_epoch}
             AND {}",
            surface.eligible_extra
        ),
        "archived_count",
        now_epoch,
    )?;
    let sql = format!(
        "UPDATE {table}
         SET archived_at_epoch = ?1,
             updated_at_epoch = ?1
         WHERE {failed_predicate}
           AND archived_at_epoch IS NULL
           AND COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch) < ?2
           AND {eligible_extra}",
        table = surface.table,
        failed_predicate = surface.failed_predicate,
        eligible_extra = surface.eligible_extra
    );
    conn.execute(&sql, params![now_epoch, cutoff_epoch])
        .with_context(|| format!("archive failure surface {}", surface.surface))
}

pub(super) fn purge_simple_surface(
    conn: &Connection,
    surface: &str,
    table: &str,
    failed_predicate: &str,
    cutoff_epoch: i64,
    now_epoch: i64,
) -> Result<usize> {
    rollup_surface(
        conn,
        surface,
        table,
        failed_predicate,
        &format!("archived_at_epoch IS NOT NULL AND archived_at_epoch < {cutoff_epoch}"),
        "purged_count",
        now_epoch,
    )?;
    let sql = format!(
        "DELETE FROM {table}
         WHERE {failed_predicate}
           AND archived_at_epoch IS NOT NULL
           AND archived_at_epoch < ?1"
    );
    conn.execute(&sql, [cutoff_epoch])
        .with_context(|| format!("purge archived failure surface {surface}"))
}

pub(super) fn purge_archived_replay_ranges(
    conn: &Connection,
    cutoff_epoch: i64,
    now_epoch: i64,
) -> Result<usize> {
    let range_ids = archived_replay_range_ids(conn, cutoff_epoch)?;
    if range_ids.is_empty() {
        return Ok(0);
    }
    rollup_ids(
        conn,
        "extraction_replay_range",
        "extraction_replay_ranges",
        &range_ids,
        "purged_count",
        now_epoch,
    )?;
    let placeholders = id_placeholders(range_ids.len(), 1);
    let sql = format!(
        "UPDATE extraction_tasks
         SET replay_range_id = NULL
         WHERE replay_range_id IN ({placeholders})"
    );
    conn.execute(&sql, rusqlite::params_from_iter(range_ids.iter()))?;
    let sql = format!("DELETE FROM extraction_replay_ranges WHERE id IN ({placeholders})");
    conn.execute(&sql, rusqlite::params_from_iter(range_ids.iter()))
        .context("purge archived extraction replay ranges")
}

pub(super) fn purge_archived_extraction_tasks(
    conn: &Connection,
    cutoff_epoch: i64,
    now_epoch: i64,
) -> Result<usize> {
    let task_ids = purgeable_extraction_task_ids(conn, cutoff_epoch)?;
    if task_ids.is_empty() {
        return Ok(0);
    }
    rollup_ids(
        conn,
        "extraction_task",
        "extraction_tasks",
        &task_ids,
        "purged_count",
        now_epoch,
    )?;
    let placeholders = id_placeholders(task_ids.len(), 1);
    let sql = format!("DELETE FROM extraction_tasks WHERE id IN ({placeholders})");
    conn.execute(&sql, rusqlite::params_from_iter(task_ids.iter()))
        .context("purge archived extraction tasks")
}

fn rollup_surface(
    conn: &Connection,
    surface: &str,
    table: &str,
    failed_predicate: &str,
    extra_predicate: &str,
    count_column: &str,
    now_epoch: i64,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO failure_lifecycle_daily
            (day_epoch, surface, failure_class, {count_column},
             oldest_failed_at_epoch, newest_failed_at_epoch, last_rollup_epoch)
         SELECT
            (COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch) / 86400) * 86400,
            ?1,
            COALESCE(failure_class, 'transient'),
            COUNT(*),
            MIN(COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch)),
            MAX(COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch)),
            ?2
         FROM {table}
         WHERE {failed_predicate}
           AND {extra_predicate}
         GROUP BY
            (COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch) / 86400) * 86400,
            COALESCE(failure_class, 'transient')
         ON CONFLICT(day_epoch, surface, failure_class) DO UPDATE SET
            {count_column} = {count_column} + excluded.{count_column},
            oldest_failed_at_epoch = CASE
                WHEN failure_lifecycle_daily.oldest_failed_at_epoch IS NULL THEN excluded.oldest_failed_at_epoch
                WHEN excluded.oldest_failed_at_epoch IS NULL THEN failure_lifecycle_daily.oldest_failed_at_epoch
                ELSE MIN(failure_lifecycle_daily.oldest_failed_at_epoch, excluded.oldest_failed_at_epoch)
            END,
            newest_failed_at_epoch = CASE
                WHEN failure_lifecycle_daily.newest_failed_at_epoch IS NULL THEN excluded.newest_failed_at_epoch
                WHEN excluded.newest_failed_at_epoch IS NULL THEN failure_lifecycle_daily.newest_failed_at_epoch
                ELSE MAX(failure_lifecycle_daily.newest_failed_at_epoch, excluded.newest_failed_at_epoch)
            END,
            last_rollup_epoch = excluded.last_rollup_epoch"
    );
    conn.execute(&sql, params![surface, now_epoch])?;
    Ok(())
}

fn rollup_ids(
    conn: &Connection,
    surface: &str,
    table: &str,
    ids: &[i64],
    count_column: &str,
    now_epoch: i64,
) -> Result<()> {
    let placeholders = id_placeholders(ids.len(), 3);
    let sql = format!(
        "INSERT INTO failure_lifecycle_daily
            (day_epoch, surface, failure_class, {count_column},
             oldest_failed_at_epoch, newest_failed_at_epoch, last_rollup_epoch)
         SELECT
            (COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch) / 86400) * 86400,
            ?1,
            COALESCE(failure_class, 'transient'),
            COUNT(*),
            MIN(COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch)),
            MAX(COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch)),
            ?2
         FROM {table}
         WHERE id IN ({placeholders})
         GROUP BY
            (COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch) / 86400) * 86400,
            COALESCE(failure_class, 'transient')
         ON CONFLICT(day_epoch, surface, failure_class) DO UPDATE SET
            {count_column} = {count_column} + excluded.{count_column},
            oldest_failed_at_epoch = CASE
                WHEN failure_lifecycle_daily.oldest_failed_at_epoch IS NULL THEN excluded.oldest_failed_at_epoch
                WHEN excluded.oldest_failed_at_epoch IS NULL THEN failure_lifecycle_daily.oldest_failed_at_epoch
                ELSE MIN(failure_lifecycle_daily.oldest_failed_at_epoch, excluded.oldest_failed_at_epoch)
            END,
            newest_failed_at_epoch = CASE
                WHEN failure_lifecycle_daily.newest_failed_at_epoch IS NULL THEN excluded.newest_failed_at_epoch
                WHEN excluded.newest_failed_at_epoch IS NULL THEN failure_lifecycle_daily.newest_failed_at_epoch
                ELSE MAX(failure_lifecycle_daily.newest_failed_at_epoch, excluded.newest_failed_at_epoch)
            END,
            last_rollup_epoch = excluded.last_rollup_epoch"
    );
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(surface.to_string()), Box::new(now_epoch)];
    values.extend(
        ids.iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>),
    );
    let refs = crate::db::to_sql_refs(&values);
    conn.execute(&sql, refs.as_slice())?;
    Ok(())
}
