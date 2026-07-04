use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

mod maintenance;
mod query;
mod sql;

#[cfg(test)]
mod tests;

use maintenance::{
    archive_surface, purge_archived_extraction_tasks, purge_archived_replay_ranges,
    purge_simple_surface, requeue_due_extraction_tasks, requeue_due_jobs,
    retry_due_extraction_replay_ranges, ArchiveSurface,
};
use query::{query_surface_stats, SurfaceQuery};
use sql::{
    count_archived_rows, count_purgeable_extraction_tasks, cutoff_epoch, failure_columns_available,
};

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
