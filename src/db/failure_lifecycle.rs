use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

pub const FAILURE_RETENTION_DAYS: i64 = 14;
pub const ARCHIVED_FAILURE_PURGE_DAYS: i64 = 90;
pub const MAX_FAILURE_AUTO_RETRIES: i64 = 3;

const SECONDS_PER_DAY: i64 = 86_400;
const FAILURE_RETRY_BASE_SECS: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    Transient,
    Permanent,
}

impl FailureClass {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureClass::Transient => "transient",
            FailureClass::Permanent => "permanent",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FailureLifecycleStats {
    pub pending_observation: FailureSurfaceStats,
    pub extraction_task: FailureSurfaceStats,
    pub extraction_replay_range: FailureSurfaceStats,
    pub job: FailureSurfaceStats,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FailureSurfaceStats {
    pub actionable_total: i64,
    pub actionable_7d: i64,
    pub transient: i64,
    pub permanent: i64,
    pub exhausted: i64,
    pub archived: i64,
    pub historical_archived: i64,
    pub historical_purged: i64,
    pub oldest_actionable_epoch: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FailureLifecycleMaintenance {
    pub retried_extraction_replay_ranges: usize,
    pub retried_extraction_tasks: usize,
    pub retried_jobs: usize,
    pub archived_pending_observations: usize,
    pub archived_extraction_tasks: usize,
    pub archived_extraction_replay_ranges: usize,
    pub archived_jobs: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ArchivedFailurePurgePlan {
    pub pending_observations: usize,
    pub extraction_replay_ranges: usize,
    pub extraction_tasks: usize,
    pub jobs: usize,
}

pub fn classify_failure(error: &str) -> FailureClass {
    let lower = error.to_ascii_lowercase();
    if [
        "schema",
        "vocab",
        "malformed",
        "invalid payload",
        "invalid json",
        "unsupported version",
        "missing evidence",
        "not implemented",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return FailureClass::Permanent;
    }

    FailureClass::Transient
}

pub fn query_failure_lifecycle_stats(
    conn: &Connection,
    now_epoch: i64,
) -> Result<FailureLifecycleStats> {
    Ok(FailureLifecycleStats {
        pending_observation: query_surface_stats(
            conn,
            SurfaceQuery {
                surface: "pending_observation",
                table: "pending_observations",
                failed_predicate: "status = 'failed'",
                attempt_column: "attempt_count",
                created_column: "created_at_epoch",
                updated_column: "updated_at_epoch",
            },
            now_epoch,
        )?,
        extraction_task: query_surface_stats(
            conn,
            SurfaceQuery {
                surface: "extraction_task",
                table: "extraction_tasks",
                failed_predicate: "status = 'failed'",
                attempt_column: "attempts",
                created_column: "created_at_epoch",
                updated_column: "updated_at_epoch",
            },
            now_epoch,
        )?,
        extraction_replay_range: query_surface_stats(
            conn,
            SurfaceQuery {
                surface: "extraction_replay_range",
                table: "extraction_replay_ranges",
                failed_predicate: "status IN ('pending', 'failed')",
                attempt_column: "attempts",
                created_column: "created_at_epoch",
                updated_column: "updated_at_epoch",
            },
            now_epoch,
        )?,
        job: query_surface_stats(
            conn,
            SurfaceQuery {
                surface: "job",
                table: "jobs",
                failed_predicate: "state = 'failed'",
                attempt_column: "attempt_count",
                created_column: "created_at_epoch",
                updated_column: "updated_at_epoch",
            },
            now_epoch,
        )?,
    })
}

pub fn maintain_failure_lifecycle(conn: &Connection) -> Result<FailureLifecycleMaintenance> {
    if !failure_columns_available(conn)? {
        return Ok(FailureLifecycleMaintenance::default());
    }
    let now = chrono::Utc::now().timestamp();
    let mut result = FailureLifecycleMaintenance {
        retried_extraction_replay_ranges: retry_due_extraction_replay_ranges(conn, now)?,
        retried_extraction_tasks: requeue_due_extraction_tasks(conn, now)?,
        retried_jobs: requeue_due_jobs(conn, now)?,
        ..FailureLifecycleMaintenance::default()
    };
    let archived = archive_eligible_failures(conn, now, FAILURE_RETENTION_DAYS)?;
    result.archived_pending_observations = archived.pending_observations;
    result.archived_extraction_tasks = archived.extraction_tasks;
    result.archived_extraction_replay_ranges = archived.extraction_replay_ranges;
    result.archived_jobs = archived.jobs;

    if result.retried_extraction_replay_ranges > 0
        || result.retried_extraction_tasks > 0
        || result.retried_jobs > 0
        || result.archived_pending_observations > 0
        || result.archived_extraction_tasks > 0
        || result.archived_extraction_replay_ranges > 0
        || result.archived_jobs > 0
    {
        crate::log::info(
            "failure_lifecycle",
            &format!(
                "maintenance retried replay_ranges={} extraction_tasks={} jobs={} archived pending_observations={} extraction_tasks={} replay_ranges={} jobs={}",
                result.retried_extraction_replay_ranges,
                result.retried_extraction_tasks,
                result.retried_jobs,
                result.archived_pending_observations,
                result.archived_extraction_tasks,
                result.archived_extraction_replay_ranges,
                result.archived_jobs
            ),
        );
    }

    Ok(result)
}

pub fn archive_eligible_failures(
    conn: &Connection,
    now_epoch: i64,
    retention_days: i64,
) -> Result<ArchivedFailurePurgePlan> {
    if !failure_columns_available(conn)? {
        return Ok(ArchivedFailurePurgePlan::default());
    }
    let cutoff = cutoff_epoch(now_epoch, retention_days);
    let pending = archive_surface(
        conn,
        ArchiveSurface {
            surface: "pending_observation",
            table: "pending_observations",
            failed_predicate: "status = 'failed'",
            eligible_extra: "1 = 1",
        },
        cutoff,
        now_epoch,
    )?;
    let extraction_tasks = archive_surface(
        conn,
        ArchiveSurface {
            surface: "extraction_task",
            table: "extraction_tasks",
            failed_predicate: "status = 'failed'",
            eligible_extra: "(failure_class = 'permanent' OR attempts >= 3)",
        },
        cutoff,
        now_epoch,
    )?;
    let replay_ranges = archive_surface(
        conn,
        ArchiveSurface {
            surface: "extraction_replay_range",
            table: "extraction_replay_ranges",
            failed_predicate: "status IN ('pending', 'failed', 'quarantined')",
            eligible_extra: "(failure_class = 'permanent' OR attempts >= 3)",
        },
        cutoff,
        now_epoch,
    )?;
    let jobs = archive_surface(
        conn,
        ArchiveSurface {
            surface: "job",
            table: "jobs",
            failed_predicate: "state = 'failed'",
            eligible_extra: "(failure_class = 'permanent' OR attempt_count >= 3)",
        },
        cutoff,
        now_epoch,
    )?;
    Ok(ArchivedFailurePurgePlan {
        pending_observations: pending,
        extraction_replay_ranges: replay_ranges,
        extraction_tasks,
        jobs,
    })
}

pub fn count_archived_failures_to_purge_at(
    conn: &Connection,
    now_epoch: i64,
    horizon_days: i64,
) -> Result<ArchivedFailurePurgePlan> {
    if !failure_columns_available(conn)? {
        return Ok(ArchivedFailurePurgePlan::default());
    }
    let cutoff = cutoff_epoch(now_epoch, horizon_days);
    Ok(ArchivedFailurePurgePlan {
        pending_observations: count_archived_rows(
            conn,
            "pending_observations",
            "status = 'failed'",
            cutoff,
        )?,
        extraction_replay_ranges: count_archived_rows(
            conn,
            "extraction_replay_ranges",
            "status IN ('pending', 'failed', 'quarantined')",
            cutoff,
        )?,
        extraction_tasks: count_purgeable_extraction_tasks(conn, cutoff)?,
        jobs: count_archived_rows(conn, "jobs", "state = 'failed'", cutoff)?,
    })
}

pub fn purge_archived_failures_at(
    conn: &Connection,
    now_epoch: i64,
    horizon_days: i64,
) -> Result<ArchivedFailurePurgePlan> {
    if !failure_columns_available(conn)? {
        return Ok(ArchivedFailurePurgePlan::default());
    }
    let cutoff = cutoff_epoch(now_epoch, horizon_days);
    let tx = conn.unchecked_transaction()?;
    let pending_observations = purge_simple_surface(
        &tx,
        "pending_observation",
        "pending_observations",
        "status = 'failed'",
        cutoff,
        now_epoch,
    )?;
    let extraction_replay_ranges = purge_archived_replay_ranges(&tx, cutoff, now_epoch)?;
    let extraction_tasks = purge_archived_extraction_tasks(&tx, cutoff, now_epoch)?;
    let jobs = purge_simple_surface(&tx, "job", "jobs", "state = 'failed'", cutoff, now_epoch)?;
    tx.commit()?;
    Ok(ArchivedFailurePurgePlan {
        pending_observations,
        extraction_replay_ranges,
        extraction_tasks,
        jobs,
    })
}

#[derive(Debug, Clone, Copy)]
struct SurfaceQuery {
    surface: &'static str,
    table: &'static str,
    failed_predicate: &'static str,
    attempt_column: &'static str,
    created_column: &'static str,
    updated_column: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct ArchiveSurface {
    surface: &'static str,
    table: &'static str,
    failed_predicate: &'static str,
    eligible_extra: &'static str,
}

fn query_surface_stats(
    conn: &Connection,
    query: SurfaceQuery,
    now_epoch: i64,
) -> Result<FailureSurfaceStats> {
    if !table_exists(conn, query.table)? {
        return Ok(FailureSurfaceStats::default());
    }
    let has_lifecycle = column_exists(conn, query.table, "archived_at_epoch")?;
    let attempt_expr = if column_exists(conn, query.table, query.attempt_column)? {
        query.attempt_column
    } else {
        "0"
    };
    let updated_expr = if column_exists(conn, query.table, query.updated_column)? {
        query.updated_column
    } else {
        "0"
    };
    let created_expr = if column_exists(conn, query.table, query.created_column)? {
        query.created_column
    } else {
        "0"
    };
    let class_expr = if has_lifecycle {
        "COALESCE(failure_class, 'transient')".to_string()
    } else {
        "'transient'".to_string()
    };
    let failed_at_expr = if has_lifecycle {
        format!(
            "COALESCE(failed_at_epoch, NULLIF({updated}, 0), {created})",
            updated = updated_expr,
            created = created_expr
        )
    } else {
        format!(
            "COALESCE(NULLIF({updated}, 0), {created})",
            updated = updated_expr,
            created = created_expr
        )
    };
    let archived_filter = if has_lifecycle {
        "archived_at_epoch IS NULL"
    } else {
        "1 = 1"
    };
    let week_ago = now_epoch.saturating_sub(7 * SECONDS_PER_DAY);
    let sql = format!(
        "SELECT
            COUNT(*) AS actionable_total,
            COALESCE(SUM(CASE WHEN {failed_at_expr} >= ?1 THEN 1 ELSE 0 END), 0) AS actionable_7d,
            COALESCE(SUM(CASE WHEN {class_expr} = 'transient' THEN 1 ELSE 0 END), 0) AS transient,
            COALESCE(SUM(CASE WHEN {class_expr} = 'permanent' THEN 1 ELSE 0 END), 0) AS permanent,
            COALESCE(SUM(CASE WHEN {attempt_col} >= ?2 THEN 1 ELSE 0 END), 0) AS exhausted,
            MIN({failed_at_expr}) AS oldest_actionable_epoch
         FROM {table}
         WHERE {failed_predicate}
           AND {archived_filter}",
        attempt_col = attempt_expr,
        table = query.table,
        failed_predicate = query.failed_predicate
    );
    let (actionable_total, actionable_7d, transient, permanent, exhausted, oldest): (
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<i64>,
    ) = conn.query_row(&sql, params![week_ago, MAX_FAILURE_AUTO_RETRIES], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        ))
    })?;
    let archived = if has_lifecycle {
        let sql = format!(
            "SELECT COUNT(*) FROM {table}
             WHERE {failed_predicate} AND archived_at_epoch IS NOT NULL",
            table = query.table,
            failed_predicate = query.failed_predicate
        );
        conn.query_row(&sql, [], |row| row.get(0))?
    } else {
        0
    };
    let (historical_archived, historical_purged) = query_historical_counts(conn, query.surface)?;
    Ok(FailureSurfaceStats {
        actionable_total,
        actionable_7d,
        transient,
        permanent,
        exhausted,
        archived,
        historical_archived,
        historical_purged,
        oldest_actionable_epoch: oldest,
    })
}

fn query_historical_counts(conn: &Connection, surface: &str) -> Result<(i64, i64)> {
    if !table_exists(conn, "failure_lifecycle_daily")? {
        return Ok((0, 0));
    }
    conn.query_row(
        "SELECT COALESCE(SUM(archived_count), 0), COALESCE(SUM(purged_count), 0)
         FROM failure_lifecycle_daily
         WHERE surface = ?1",
        [surface],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(Into::into)
}

fn retry_due_extraction_replay_ranges(conn: &Connection, now_epoch: i64) -> Result<usize> {
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

fn requeue_due_extraction_tasks(conn: &Connection, now_epoch: i64) -> Result<usize> {
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

fn requeue_due_jobs(conn: &Connection, now_epoch: i64) -> Result<usize> {
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

fn archive_surface(
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

fn purge_simple_surface(
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

fn purge_archived_replay_ranges(
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

fn purge_archived_extraction_tasks(
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

fn count_archived_rows(
    conn: &Connection,
    table: &str,
    failed_predicate: &str,
    cutoff_epoch: i64,
) -> Result<usize> {
    let sql = format!(
        "SELECT COUNT(*) FROM {table}
         WHERE {failed_predicate}
           AND archived_at_epoch IS NOT NULL
           AND archived_at_epoch < ?1"
    );
    let count: i64 = conn.query_row(&sql, [cutoff_epoch], |row| row.get(0))?;
    Ok(count.max(0) as usize)
}

fn count_purgeable_extraction_tasks(conn: &Connection, cutoff_epoch: i64) -> Result<usize> {
    Ok(purgeable_extraction_task_ids(conn, cutoff_epoch)?.len())
}

fn archived_replay_range_ids(conn: &Connection, cutoff_epoch: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id
         FROM extraction_replay_ranges
         WHERE status IN ('pending', 'failed', 'quarantined')
           AND archived_at_epoch IS NOT NULL
           AND archived_at_epoch < ?1
         ORDER BY archived_at_epoch ASC, id ASC",
    )?;
    let rows = stmt.query_map([cutoff_epoch], |row| row.get::<_, i64>(0))?;
    collect_i64_rows(rows)
}

fn purgeable_extraction_task_ids(conn: &Connection, cutoff_epoch: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT t.id
         FROM extraction_tasks t
         WHERE t.status = 'failed'
           AND t.archived_at_epoch IS NOT NULL
           AND t.archived_at_epoch < ?1
           AND NOT EXISTS (
             SELECT 1 FROM extraction_replay_ranges r
             WHERE r.source_task_id = t.id
                OR r.replay_task_id = t.id
           )
         ORDER BY t.archived_at_epoch ASC, t.id ASC",
    )?;
    let rows = stmt.query_map([cutoff_epoch], |row| row.get::<_, i64>(0))?;
    collect_i64_rows(rows)
}

fn collect_i64_rows<F>(rows: rusqlite::MappedRows<'_, F>) -> Result<Vec<i64>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<i64>,
{
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

fn id_placeholders(len: usize, start_idx: usize) -> String {
    (start_idx..start_idx + len)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn cutoff_epoch(now_epoch: i64, days: i64) -> i64 {
    now_epoch.saturating_sub(days.max(0).saturating_mul(SECONDS_PER_DAY))
}

fn failure_columns_available(conn: &Connection) -> Result<bool> {
    Ok(column_exists(conn, "jobs", "archived_at_epoch")?
        && table_exists(conn, "failure_lifecycle_daily")?)
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    if !table_exists(conn, table)? {
        return Ok(false);
    }
    let sql = format!("PRAGMA table_info({})", quote_identifier(table));
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CaptureEventInput, ExtractionTaskKind, JobType};

    #[test]
    fn classifier_maps_known_permanent_patterns() {
        for error in [
            "schema mismatch in model output",
            "malformed payload",
            "unsupported version marker",
            "missing evidence rows",
            "rule candidate extraction is not implemented",
        ] {
            assert_eq!(classify_failure(error), FailureClass::Permanent);
        }
    }

    #[test]
    fn classifier_defaults_unknown_to_transient() {
        assert_eq!(
            classify_failure("the model returned a strange error"),
            FailureClass::Transient
        );
    }

    fn setup_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn seed_pending_failure(conn: &Connection, failed_at: i64, class: &str) -> Result<i64> {
        let id = crate::db::enqueue_pending(
            conn,
            "codex-cli",
            "sess-failure",
            "/tmp/remem",
            "Bash",
            Some("{}"),
            Some("{}"),
            Some("/tmp/remem"),
        )?;
        conn.execute(
            "UPDATE pending_observations
             SET status = 'failed',
                 failure_class = ?1,
                 failed_at_epoch = ?2,
                 updated_at_epoch = ?2
             WHERE id = ?3",
            params![class, failed_at, id],
        )?;
        Ok(id)
    }

    fn seed_job_failure(
        conn: &Connection,
        failed_at: i64,
        class: &str,
        attempts: i64,
    ) -> Result<i64> {
        let id = crate::db::enqueue_job(
            conn,
            "codex-cli",
            JobType::Summary,
            "/tmp/remem",
            Some("sess-failure"),
            "{}",
            100,
        )?;
        conn.execute(
            "UPDATE jobs
             SET state = 'failed',
                 attempt_count = ?1,
                 failure_class = ?2,
                 failed_at_epoch = ?3,
                 updated_at_epoch = ?3,
                 next_retry_epoch = ?3
             WHERE id = ?4",
            params![attempts, class, failed_at, id],
        )?;
        Ok(id)
    }

    fn seed_extraction_task(conn: &Connection) -> Result<(i64, i64)> {
        let outcome = crate::db::record_captured_event(
            conn,
            &CaptureEventInput {
                host: "codex-cli",
                session_id: "sess-extraction",
                project: "/tmp/remem",
                cwd: Some("/tmp/remem"),
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: r#"{"tool_name":"Bash"}"#,
                task_kind: Some(ExtractionTaskKind::ObservationExtract),
            },
        )?;
        let task_id = outcome
            .extraction_task_id
            .expect("capture should coalesce extraction task");
        Ok((task_id, outcome.event_row_id))
    }

    #[test]
    fn archive_moves_old_failures_out_of_actionable_stats() -> Result<()> {
        let conn = setup_conn()?;
        let now = 2_000_000;
        let old = now - 20 * SECONDS_PER_DAY;
        seed_pending_failure(&conn, old, "permanent")?;

        let before = query_failure_lifecycle_stats(&conn, now)?;
        assert_eq!(before.pending_observation.actionable_total, 1);

        let archived = archive_eligible_failures(&conn, now, FAILURE_RETENTION_DAYS)?;
        assert_eq!(archived.pending_observations, 1);

        let after = query_failure_lifecycle_stats(&conn, now)?;
        assert_eq!(after.pending_observation.actionable_total, 0);
        assert_eq!(after.pending_observation.archived, 1);
        assert_eq!(after.pending_observation.historical_archived, 1);
        Ok(())
    }

    #[test]
    fn maintenance_requeues_due_transient_job_failure() -> Result<()> {
        let conn = setup_conn()?;
        let now = chrono::Utc::now().timestamp();
        let job_id = seed_job_failure(&conn, now - 1_000, "transient", 0)?;

        let result = maintain_failure_lifecycle(&conn)?;

        assert_eq!(result.retried_jobs, 1);
        let (state, failure_class, failed_at): (String, Option<String>, Option<i64>) = conn
            .query_row(
                "SELECT state, failure_class, failed_at_epoch FROM jobs WHERE id = ?1",
                [job_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        assert_eq!(state, "pending");
        assert_eq!(failure_class, None);
        assert_eq!(failed_at, None);
        Ok(())
    }

    #[test]
    fn maintenance_requeues_due_no_range_extraction_task_failure() -> Result<()> {
        let conn = setup_conn()?;
        let now = chrono::Utc::now().timestamp();
        let (task_id, _) = seed_extraction_task(&conn)?;
        conn.execute(
            "UPDATE extraction_tasks
             SET status = 'failed',
                 attempts = 0,
                 failure_class = 'transient',
                 failed_at_epoch = ?1,
                 updated_at_epoch = ?1
             WHERE id = ?2",
            params![now - 1_000, task_id],
        )?;

        let result = maintain_failure_lifecycle(&conn)?;

        assert_eq!(result.retried_extraction_tasks, 1);
        let (status, failure_class, failed_at): (String, Option<String>, Option<i64>) = conn
            .query_row(
                "SELECT status, failure_class, failed_at_epoch FROM extraction_tasks WHERE id = ?1",
                [task_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        assert_eq!(status, "pending");
        assert_eq!(failure_class, None);
        assert_eq!(failed_at, None);
        Ok(())
    }

    #[test]
    fn maintenance_requeues_due_transient_replay_range() -> Result<()> {
        let conn = setup_conn()?;
        let now = chrono::Utc::now().timestamp();
        let (task_id, event_row_id) = seed_extraction_task(&conn)?;
        let (task_kind, host_id, workspace_id, project_id, session_row_id): (
            String,
            i64,
            i64,
            i64,
            Option<i64>,
        ) = conn.query_row(
            "SELECT task_kind, host_id, workspace_id, project_id, session_row_id
             FROM extraction_tasks
             WHERE id = ?1",
            [task_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
        conn.execute(
            "INSERT INTO extraction_replay_ranges
             (source_task_id, task_kind, host_id, workspace_id, project_id, session_row_id,
              from_event_id, to_event_id, status, attempts, last_error, failure_class,
              failed_at_epoch, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 'pending', 0,
                     'transient timeout', 'transient', ?8, ?8, ?8)",
            params![
                task_id,
                task_kind,
                host_id,
                workspace_id,
                project_id,
                session_row_id,
                event_row_id,
                now - 1_000
            ],
        )?;
        let range_id = conn.last_insert_rowid();

        let result = maintain_failure_lifecycle(&conn)?;

        assert_eq!(result.retried_extraction_replay_ranges, 1);
        let (status, attempts, failure_class, failed_at): (
            String,
            i64,
            Option<String>,
            Option<i64>,
        ) = conn.query_row(
            "SELECT status, attempts, failure_class, failed_at_epoch
             FROM extraction_replay_ranges WHERE id = ?1",
            [range_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(status, "requeued");
        assert_eq!(attempts, 1);
        assert_eq!(failure_class, None);
        assert_eq!(failed_at, None);
        let replay_tasks: i64 = conn.query_row(
            "SELECT COUNT(*) FROM extraction_tasks
             WHERE replay_range_id = ?1 AND status = 'pending'",
            [range_id],
            |row| row.get(0),
        )?;
        assert_eq!(replay_tasks, 1);
        Ok(())
    }

    #[test]
    fn purge_archived_failures_deletes_only_explicit_old_archives() -> Result<()> {
        let conn = setup_conn()?;
        let now = 5_000_000;
        let old = now - 120 * SECONDS_PER_DAY;
        let pending_id = seed_pending_failure(&conn, old, "permanent")?;
        archive_eligible_failures(&conn, now - 100 * SECONDS_PER_DAY, FAILURE_RETENTION_DAYS)?;
        conn.execute(
            "UPDATE pending_observations SET archived_at_epoch = ?1 WHERE id = ?2",
            params![old, pending_id],
        )?;

        let plan = count_archived_failures_to_purge_at(&conn, now, 90)?;
        assert_eq!(plan.pending_observations, 1);

        let purged = purge_archived_failures_at(&conn, now, 90)?;
        assert_eq!(purged.pending_observations, 1);
        let remaining: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pending_observations WHERE id = ?1",
            [pending_id],
            |row| row.get(0),
        )?;
        assert_eq!(remaining, 0);
        let history = query_failure_lifecycle_stats(&conn, now)?;
        assert_eq!(history.pending_observation.historical_purged, 1);
        Ok(())
    }
}
