use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;

use crate::{db, memory, workstream};

const CLEANUP_POLICY_VERSION: i64 = 1;
const FAILURE_ERROR_LIMIT_BYTES: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupPolicy {
    archived_failure_days: Option<i64>,
}

impl CleanupPolicy {
    pub fn automatic() -> Self {
        Self {
            archived_failure_days: None,
        }
    }

    pub fn manual(archived_failure_days: Option<i64>) -> Result<Self> {
        if archived_failure_days.is_some_and(|days| days <= 0) {
            bail!("--archived-failures must be a positive number of days");
        }
        Ok(Self {
            archived_failure_days,
        })
    }

    pub fn retention_days(self) -> CleanupRetentionDays {
        CleanupRetentionDays {
            old_events: memory::OLD_EVENT_RETENTION_DAYS,
            compressed_source_observations: memory::COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
            stale_memories: memory::STALE_MEMORY_ARCHIVE_DAYS,
            archived_failures: self
                .archived_failure_days
                .unwrap_or(db::ARCHIVED_FAILURE_PURGE_DAYS),
            workstream_auto_pause: workstream::DEFAULT_AUTO_PAUSE_DAYS,
            workstream_auto_abandon: workstream::DEFAULT_AUTO_ABANDON_DAYS,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CleanupRetentionDays {
    pub old_events: i64,
    pub compressed_source_observations: i64,
    pub stale_memories: i64,
    pub archived_failures: i64,
    pub workstream_auto_pause: i64,
    pub workstream_auto_abandon: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CleanupPlan {
    pub expired_memories_to_stale: usize,
    pub inactive_workstreams_to_pause: usize,
    pub long_paused_workstreams_to_abandon: usize,
    pub old_events_to_delete: usize,
    pub compressed_source_observations_to_delete: usize,
    pub stale_memories_to_archive: usize,
    pub archived_failures_to_purge: db::ArchivedFailurePurgePlan,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CleanupApplied {
    pub expired_memories_marked_stale: usize,
    pub inactive_workstreams_paused: usize,
    pub long_paused_workstreams_abandoned: usize,
    pub old_events_deleted: usize,
    pub compressed_source_observations_deleted: usize,
    pub stale_memories_archived: usize,
    pub archived_failures_purged: db::ArchivedFailurePurgePlan,
}

impl CleanupApplied {
    fn matches_plan(&self, plan: &CleanupPlan) -> bool {
        self.expired_memories_marked_stale == plan.expired_memories_to_stale
            && self.inactive_workstreams_paused == plan.inactive_workstreams_to_pause
            && self.long_paused_workstreams_abandoned == plan.long_paused_workstreams_to_abandon
            && self.old_events_deleted == plan.old_events_to_delete
            && self.compressed_source_observations_deleted
                == plan.compressed_source_observations_to_delete
            && self.stale_memories_archived == plan.stale_memories_to_archive
            && self.archived_failures_purged == plan.archived_failures_to_purge
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CleanupExecution {
    pub plan: CleanupPlan,
    pub applied: CleanupApplied,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CleanupReport {
    pub dry_run: bool,
    pub retention_days: CleanupRetentionDays,
    pub plan: CleanupPlan,
    pub applied: Option<CleanupApplied>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupRun {
    pub id: i64,
    pub job_id: Option<i64>,
    pub started_at_epoch: i64,
    pub finished_at_epoch: i64,
    pub outcome: String,
    pub counts_json: Option<String>,
    pub error: Option<String>,
}

pub fn preview_cleanup(
    conn: &Connection,
    now_epoch: i64,
    policy: CleanupPolicy,
) -> Result<CleanupPlan> {
    build_cleanup_plan(conn, now_epoch, policy)
}

pub fn execute_manual_cleanup(
    conn: &Connection,
    now_epoch: i64,
    policy: CleanupPolicy,
) -> Result<CleanupExecution> {
    execute_cleanup(conn, now_epoch, policy, CleanupTrigger::Manual)
}

pub fn execute_automatic_cleanup_job(
    conn: &Connection,
    job_id: i64,
    lease_owner: &str,
    now_epoch: i64,
) -> Result<CleanupExecution> {
    execute_cleanup(
        conn,
        now_epoch,
        CleanupPolicy::automatic(),
        CleanupTrigger::Automatic {
            job_id,
            lease_owner,
        },
    )
}

pub fn latest_automatic_cleanup_run(
    conn: &Connection,
    outcome: &str,
) -> Result<Option<CleanupRun>> {
    if !matches!(outcome, "success" | "failure") {
        bail!("cleanup run outcome must be success or failure");
    }
    conn.query_row(
        "SELECT id, job_id, started_at_epoch, finished_at_epoch, outcome,
                counts_json, error
         FROM maintenance_runs
         WHERE \"trigger\" = 'automatic' AND outcome = ?1
         ORDER BY finished_at_epoch DESC, id DESC
         LIMIT 1",
        params![outcome],
        |row| {
            Ok(CleanupRun {
                id: row.get(0)?,
                job_id: row.get(1)?,
                started_at_epoch: row.get(2)?,
                finished_at_epoch: row.get(3)?,
                outcome: row.get(4)?,
                counts_json: row.get(5)?,
                error: row.get(6)?,
            })
        },
    )
    .optional()
    .context("read latest automatic cleanup run")
}

#[derive(Clone, Copy)]
enum CleanupTrigger<'a> {
    Manual,
    Automatic { job_id: i64, lease_owner: &'a str },
}

impl CleanupTrigger<'_> {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Automatic { .. } => "automatic",
        }
    }

    fn job_id(self) -> Option<i64> {
        match self {
            Self::Manual => None,
            Self::Automatic { job_id, .. } => Some(job_id),
        }
    }
}

fn execute_cleanup(
    conn: &Connection,
    now_epoch: i64,
    policy: CleanupPolicy,
    trigger: CleanupTrigger<'_>,
) -> Result<CleanupExecution> {
    let tx = match Transaction::new_unchecked(conn, TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(error) => {
            let error = anyhow::Error::from(error).context("begin cleanup transaction");
            return Err(record_failure_after_rollback(
                conn, trigger, now_epoch, error,
            ));
        }
    };
    let execution = execute_cleanup_in_transaction(&tx, now_epoch, policy, trigger);
    let execution = match execution {
        Ok(execution) => execution,
        Err(error) => {
            drop(tx);
            return Err(record_failure_after_rollback(
                conn, trigger, now_epoch, error,
            ));
        }
    };
    if let Err(error) = tx.commit().context("commit cleanup transaction") {
        return Err(record_failure_after_rollback(
            conn, trigger, now_epoch, error,
        ));
    }
    Ok(execution)
}

fn execute_cleanup_in_transaction(
    conn: &Connection,
    started_at_epoch: i64,
    policy: CleanupPolicy,
    trigger: CleanupTrigger<'_>,
) -> Result<CleanupExecution> {
    if let CleanupTrigger::Automatic {
        job_id,
        lease_owner,
    } = trigger
    {
        validate_automatic_job(conn, job_id, lease_owner, started_at_epoch)?;
    }
    let plan = build_cleanup_plan(conn, started_at_epoch, policy)?;
    let applied = apply_cleanup_plan(conn, started_at_epoch, policy)?;
    if !applied.matches_plan(&plan) {
        bail!("cleanup plan/apply count invariant failed");
    }
    let finished_at_epoch = cleanup_finished_at(started_at_epoch);
    insert_success_run(conn, trigger, started_at_epoch, finished_at_epoch, &applied)?;
    if let CleanupTrigger::Automatic {
        job_id,
        lease_owner,
    } = trigger
    {
        finish_automatic_job(
            conn,
            job_id,
            lease_owner,
            started_at_epoch,
            finished_at_epoch,
        )?;
    }
    Ok(CleanupExecution { plan, applied })
}

fn build_cleanup_plan(
    conn: &Connection,
    now_epoch: i64,
    policy: CleanupPolicy,
) -> Result<CleanupPlan> {
    Ok(CleanupPlan {
        expired_memories_to_stale: memory::lifecycle::count_expired_active_memories(
            conn, now_epoch,
        )?,
        inactive_workstreams_to_pause: workstream::count_auto_pause_all_inactive_at(
            conn,
            now_epoch,
            workstream::DEFAULT_AUTO_PAUSE_DAYS,
        )?,
        long_paused_workstreams_to_abandon: workstream::count_auto_abandon_all_inactive_at(
            conn,
            now_epoch,
            workstream::DEFAULT_AUTO_ABANDON_DAYS,
        )?,
        old_events_to_delete: memory::count_old_events_at(
            conn,
            now_epoch,
            memory::OLD_EVENT_RETENTION_DAYS,
        )?,
        compressed_source_observations_to_delete:
            memory::count_compressed_source_observations_to_delete_at(
                conn,
                now_epoch,
                memory::COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
            )?,
        stale_memories_to_archive: memory::count_stale_memories_to_archive_at(
            conn,
            now_epoch,
            memory::STALE_MEMORY_ARCHIVE_DAYS,
        )?,
        archived_failures_to_purge: match policy.archived_failure_days {
            Some(days) => db::count_archived_failures_to_purge_at(conn, now_epoch, days)?,
            None => db::ArchivedFailurePurgePlan::default(),
        },
    })
}

fn apply_cleanup_plan(
    conn: &Connection,
    now_epoch: i64,
    policy: CleanupPolicy,
) -> Result<CleanupApplied> {
    Ok(CleanupApplied {
        expired_memories_marked_stale: memory::lifecycle::expire_active_memories(conn, now_epoch)?,
        inactive_workstreams_paused: workstream::auto_pause_all_inactive_at(
            conn,
            now_epoch,
            workstream::DEFAULT_AUTO_PAUSE_DAYS,
        )?,
        long_paused_workstreams_abandoned: workstream::auto_abandon_all_inactive_at(
            conn,
            now_epoch,
            workstream::DEFAULT_AUTO_ABANDON_DAYS,
        )?,
        old_events_deleted: memory::cleanup_old_events_at(
            conn,
            now_epoch,
            memory::OLD_EVENT_RETENTION_DAYS,
        )?,
        compressed_source_observations_deleted: memory::cleanup_compressed_source_observations_at(
            conn,
            now_epoch,
            memory::COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )?,
        stale_memories_archived: memory::archive_stale_memories_at(
            conn,
            now_epoch,
            memory::STALE_MEMORY_ARCHIVE_DAYS,
        )?,
        archived_failures_purged: match policy.archived_failure_days {
            Some(days) => db::purge_archived_failures_at(conn, now_epoch, days)?,
            None => db::ArchivedFailurePurgePlan::default(),
        },
    })
}

fn validate_automatic_job(
    conn: &Connection,
    job_id: i64,
    lease_owner: &str,
    now_epoch: i64,
) -> Result<()> {
    let valid: bool = conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM jobs
           WHERE id = ?1 AND job_type = 'cleanup' AND state = 'processing'
             AND lease_owner = ?2 AND lease_expires_epoch IS NOT NULL
             AND lease_expires_epoch >= ?3
         )",
        params![job_id, lease_owner, now_epoch],
        |row| row.get(0),
    )?;
    if !valid {
        bail!("automatic cleanup job lease validation failed: job_id={job_id} owner={lease_owner}");
    }
    Ok(())
}

fn finish_automatic_job(
    conn: &Connection,
    job_id: i64,
    lease_owner: &str,
    lease_validation_epoch: i64,
    finished_at_epoch: i64,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE jobs
         SET state = 'done', lease_owner = NULL, lease_expires_epoch = NULL,
             next_retry_epoch = 0, last_error = NULL, failure_class = NULL,
             failed_at_epoch = NULL, archived_at_epoch = NULL,
             updated_at_epoch = ?1
         WHERE id = ?2 AND job_type = 'cleanup' AND state = 'processing'
           AND lease_owner = ?3 AND lease_expires_epoch IS NOT NULL
           AND lease_expires_epoch >= ?4",
        params![
            finished_at_epoch,
            job_id,
            lease_owner,
            lease_validation_epoch
        ],
    )?;
    if changed != 1 {
        bail!("automatic cleanup job completion lost its lease: job_id={job_id}");
    }
    Ok(())
}

fn insert_success_run(
    conn: &Connection,
    trigger: CleanupTrigger<'_>,
    started_at_epoch: i64,
    finished_at_epoch: i64,
    applied: &CleanupApplied,
) -> Result<()> {
    let counts_json = serde_json::to_string(applied)?;
    conn.execute(
        "INSERT INTO maintenance_runs
         (job_id, \"trigger\", policy_version, started_at_epoch,
          finished_at_epoch, outcome, counts_json, error)
         VALUES (?1, ?2, ?3, ?4, ?5, 'success', ?6, NULL)",
        params![
            trigger.job_id(),
            trigger.as_str(),
            CLEANUP_POLICY_VERSION,
            started_at_epoch,
            finished_at_epoch,
            counts_json
        ],
    )
    .context("record successful cleanup run")?;
    Ok(())
}

fn record_failure_after_rollback(
    conn: &Connection,
    trigger: CleanupTrigger<'_>,
    started_at_epoch: i64,
    error: anyhow::Error,
) -> anyhow::Error {
    let finished_at_epoch = cleanup_finished_at(started_at_epoch);
    let safe_error = safe_cleanup_error(&error);
    let ledger_result = record_failure_run(
        conn,
        trigger,
        started_at_epoch,
        finished_at_epoch,
        &safe_error,
    );
    match ledger_result {
        Ok(()) => anyhow::anyhow!(safe_error),
        Err(ledger_error) => {
            let safe_ledger_error = safe_cleanup_error(&ledger_error);
            anyhow::anyhow!(
                "{safe_error}; additionally failed to record cleanup failure: {safe_ledger_error}"
            )
        }
    }
}

fn record_failure_run(
    conn: &Connection,
    trigger: CleanupTrigger<'_>,
    started_at_epoch: i64,
    finished_at_epoch: i64,
    safe_error: &str,
) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .context("begin cleanup failure ledger transaction")?;
    let job_id = match trigger.job_id() {
        Some(job_id) if job_exists(&tx, job_id)? => Some(job_id),
        _ => None,
    };
    let stored_error = if safe_error.is_empty() {
        "cleanup transaction failed"
    } else {
        safe_error
    };
    tx.execute(
        "INSERT INTO maintenance_runs
         (job_id, \"trigger\", policy_version, started_at_epoch,
          finished_at_epoch, outcome, counts_json, error)
         VALUES (?1, ?2, ?3, ?4, ?5, 'failure', NULL, ?6)",
        params![
            job_id,
            trigger.as_str(),
            CLEANUP_POLICY_VERSION,
            started_at_epoch,
            finished_at_epoch,
            stored_error
        ],
    )
    .context("record failed cleanup run")?;
    tx.commit()
        .context("commit cleanup failure ledger transaction")?;
    Ok(())
}

fn cleanup_finished_at(started_at_epoch: i64) -> i64 {
    chrono::Utc::now().timestamp().max(started_at_epoch)
}

pub(crate) fn safe_cleanup_error(error: &anyhow::Error) -> String {
    let raw = format!("{error:#}");
    let bounded =
        crate::adapter::common::redact_hook_payload_preview(&raw, FAILURE_ERROR_LIMIT_BYTES);
    let bounded = bounded.trim();
    if bounded.is_empty() {
        "cleanup transaction failed".to_string()
    } else {
        bounded.to_string()
    }
}

fn job_exists(conn: &Connection, job_id: i64) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE id = ?1)",
        params![job_id],
        |row| row.get(0),
    )?)
}

#[cfg(test)]
mod tests;
