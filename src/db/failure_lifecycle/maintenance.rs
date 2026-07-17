use anyhow::{bail, Context, Result};
use rusqlite::{
    params, Connection, Error as SqliteError, ErrorCode, OptionalExtension, TransactionBehavior,
};

use super::sql::{archived_replay_range_ids, id_placeholders, purgeable_extraction_task_ids};
use super::{FAILURE_RETRY_BASE_SECS, MAX_FAILURE_AUTO_RETRIES};

const JOB_RECOVERY_BATCH_LIMIT: i64 = 25;
const JOB_RECOVERY_ERROR_LIMIT: usize = 2000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum JobRecoveryOutcome {
    Requeued {
        source_id: i64,
        identity_kind: crate::db::JobIdentityKind,
    },
    Coalesced {
        source_id: i64,
        canonical_id: i64,
        identity_kind: crate::db::JobIdentityKind,
    },
    SkippedRetiredSummary {
        source_id: i64,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct JobRecoveryBatch {
    pub(super) outcomes: Vec<JobRecoveryOutcome>,
    pub(super) requeued: usize,
    pub(super) coalesced: usize,
}

#[derive(Debug)]
struct JobRecoverySource {
    host: String,
    job_type: String,
    project: String,
    session_id: Option<String>,
    payload_json: String,
    priority: i64,
    last_error: Option<String>,
}

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
        crate::db::extraction_replay::enqueue_replay_extraction_task(conn, range_id, false)?;
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

pub(super) fn requeue_due_jobs(conn: &Connection, now_epoch: i64) -> Result<JobRecoveryBatch> {
    let candidates = {
        let mut statement = conn.prepare(
            "SELECT id FROM jobs
             WHERE state = 'failed'
               AND job_type <> 'summary'
               AND archived_at_epoch IS NULL
               AND COALESCE(failure_class, 'transient') = 'transient'
               AND attempt_count < ?1
               AND COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch)
                   + (?2 * (1 << attempt_count)) <= ?3
             ORDER BY COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch) ASC, id ASC
             LIMIT ?4",
        )?;
        let ids = statement
            .query_map(
                params![
                    MAX_FAILURE_AUTO_RETRIES,
                    FAILURE_RETRY_BASE_SECS,
                    now_epoch,
                    JOB_RECOVERY_BATCH_LIMIT
                ],
                |row| row.get(0),
            )?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        ids
    };
    test_after_job_candidates_collected();

    let mut batch = JobRecoveryBatch::default();
    for source_id in candidates {
        let Some(outcome) = recover_due_job_candidate(conn, source_id, now_epoch)? else {
            continue;
        };
        match &outcome {
            JobRecoveryOutcome::Requeued {
                source_id,
                identity_kind,
            } => {
                batch.requeued += 1;
                crate::log::info(
                    "failure_lifecycle",
                    &format!(
                        "job recovery requeued source_id={source_id} identity={}",
                        identity_kind.as_str()
                    ),
                );
            }
            JobRecoveryOutcome::Coalesced {
                source_id,
                canonical_id,
                identity_kind,
            } => {
                batch.coalesced += 1;
                crate::log::info(
                    "failure_lifecycle",
                    &format!(
                        "job recovery coalesced source_id={source_id} canonical_id={canonical_id} identity={}",
                        identity_kind.as_str()
                    ),
                );
            }
            JobRecoveryOutcome::SkippedRetiredSummary { source_id } => {
                crate::log::info(
                    "failure_lifecycle",
                    &format!("job recovery skipped retired_summary source_id={source_id}"),
                );
            }
        }
        batch.outcomes.push(outcome);
    }
    Ok(batch)
}

pub(super) fn recover_due_job_candidate(
    conn: &Connection,
    source_id: i64,
    now_epoch: i64,
) -> Result<Option<JobRecoveryOutcome>> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .context("begin job failure recovery transaction")?;
    let Some(source) = load_eligible_job_source(&tx, source_id, now_epoch)? else {
        tx.commit()?;
        return Ok(None);
    };
    if source.job_type == "summary" {
        tx.commit()?;
        return Ok(Some(JobRecoveryOutcome::SkippedRetiredSummary {
            source_id,
        }));
    }

    let identity_kind = job_identity_kind(&source.job_type);
    if !test_skip_initial_job_canonical_lookup() {
        if let Some(canonical_id) =
            find_active_job_canonical(&tx, &source, source_id, identity_kind)?
        {
            coalesce_failed_job_source(
                &tx,
                source_id,
                canonical_id,
                identity_kind,
                &source,
                now_epoch,
            )?;
            tx.commit()?;
            return Ok(Some(JobRecoveryOutcome::Coalesced {
                source_id,
                canonical_id,
                identity_kind,
            }));
        }
    }

    let update = tx.execute(
        "UPDATE jobs
         SET state = 'pending', lease_owner = NULL, lease_expires_epoch = NULL,
             next_retry_epoch = ?1, failure_class = NULL, failed_at_epoch = NULL,
             archived_at_epoch = NULL, updated_at_epoch = ?1
         WHERE id = ?2 AND state = 'failed' AND job_type <> 'summary'
           AND archived_at_epoch IS NULL
           AND COALESCE(failure_class, 'transient') = 'transient'
           AND attempt_count < ?3
           AND COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch)
               + (?4 * (1 << attempt_count)) <= ?1",
        params![
            now_epoch,
            source_id,
            MAX_FAILURE_AUTO_RETRIES,
            FAILURE_RETRY_BASE_SECS
        ],
    );
    match update {
        Ok(1) => {
            tx.commit()?;
            Ok(Some(JobRecoveryOutcome::Requeued {
                source_id,
                identity_kind,
            }))
        }
        Ok(0) => {
            tx.commit()?;
            Ok(None)
        }
        Ok(count) => bail!(
            "job failure recovery invariant violated: source_id={source_id} affected_rows={count}"
        ),
        Err(error) if is_job_identity_conflict(&error, identity_kind) => {
            test_before_job_canonical_reread()?;
            let canonical_id = find_active_job_canonical(
                &tx,
                &source,
                source_id,
                identity_kind,
            )?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "job recovery identity conflict had no active canonical: source_id={source_id} identity={}",
                    identity_kind.as_str()
                )
            })?;
            coalesce_failed_job_source(
                &tx,
                source_id,
                canonical_id,
                identity_kind,
                &source,
                now_epoch,
            )?;
            tx.commit()?;
            Ok(Some(JobRecoveryOutcome::Coalesced {
                source_id,
                canonical_id,
                identity_kind,
            }))
        }
        Err(error) => Err(error).context("requeue failed job source"),
    }
}

fn load_eligible_job_source(
    conn: &Connection,
    source_id: i64,
    now_epoch: i64,
) -> Result<Option<JobRecoverySource>> {
    conn.query_row(
        "SELECT host, job_type, project, session_id, payload_json, priority, last_error
         FROM jobs
         WHERE id = ?1 AND state = 'failed' AND archived_at_epoch IS NULL
           AND COALESCE(failure_class, 'transient') = 'transient'
           AND attempt_count < ?2
           AND COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch)
               + (?3 * (1 << attempt_count)) <= ?4",
        params![
            source_id,
            MAX_FAILURE_AUTO_RETRIES,
            FAILURE_RETRY_BASE_SECS,
            now_epoch
        ],
        |row| {
            Ok(JobRecoverySource {
                host: row.get(0)?,
                job_type: row.get(1)?,
                project: row.get(2)?,
                session_id: row.get(3)?,
                payload_json: row.get(4)?,
                priority: row.get(5)?,
                last_error: row.get(6)?,
            })
        },
    )
    .optional()
    .context("load eligible failed job source")
}

fn find_active_job_canonical(
    conn: &Connection,
    source: &JobRecoverySource,
    source_id: i64,
    identity_kind: crate::db::JobIdentityKind,
) -> Result<Option<i64>> {
    let canonical = match identity_kind {
        crate::db::JobIdentityKind::Ordinary => conn
            .query_row(
                "SELECT id FROM jobs
                 WHERE id <> ?1 AND host = ?2 AND job_type = ?3 AND project = ?4
                   AND COALESCE(session_id, '') = COALESCE(?5, '')
                   AND state IN ('pending', 'processing')
                 ORDER BY id ASC LIMIT 1",
                params![
                    source_id,
                    source.host,
                    source.job_type,
                    source.project,
                    source.session_id
                ],
                |row| row.get(0),
            )
            .optional(),
        crate::db::JobIdentityKind::Dream => conn
            .query_row(
                "SELECT id FROM jobs
                 WHERE id <> ?1 AND job_type = 'dream' AND project = ?2
                   AND state IN ('pending', 'processing')
                 ORDER BY id ASC LIMIT 1",
                params![source_id, source.project],
                |row| row.get(0),
            )
            .optional(),
        crate::db::JobIdentityKind::CompileRules => conn
            .query_row(
                "SELECT id FROM jobs
                 WHERE id <> ?1 AND job_type = 'compile_rules' AND project = ?2
                   AND state = 'pending'
                 ORDER BY id ASC LIMIT 1",
                params![source_id, source.project],
                |row| row.get(0),
            )
            .optional(),
    };
    canonical.context("read active job recovery canonical")
}

fn coalesce_failed_job_source(
    conn: &Connection,
    source_id: i64,
    canonical_id: i64,
    identity_kind: crate::db::JobIdentityKind,
    source: &JobRecoverySource,
    now_epoch: i64,
) -> Result<()> {
    if identity_kind == crate::db::JobIdentityKind::Dream {
        merge_dream_source_into_pending_canonical(conn, source, canonical_id, now_epoch)?;
    }
    let marker = format!("[auto_recovery_coalesced_to_canonical id={canonical_id}]");
    let last_error = bounded_job_recovery_error(source.last_error.as_deref(), &marker);
    let changed = conn.execute(
        "UPDATE jobs
         SET failure_class = 'permanent', next_retry_epoch = 0, last_error = ?1
         WHERE id = ?2 AND state = 'failed' AND job_type <> 'summary'
           AND archived_at_epoch IS NULL
           AND COALESCE(failure_class, 'transient') = 'transient'
           AND attempt_count < ?3
           AND COALESCE(failed_at_epoch, updated_at_epoch, created_at_epoch)
               + (?4 * (1 << attempt_count)) <= ?5",
        params![
            last_error,
            source_id,
            MAX_FAILURE_AUTO_RETRIES,
            FAILURE_RETRY_BASE_SECS,
            now_epoch
        ],
    )?;
    if changed != 1 {
        bail!(
            "failed job source changed before coalescing: source_id={source_id} canonical_id={canonical_id} affected_rows={changed}"
        );
    }
    Ok(())
}

fn merge_dream_source_into_pending_canonical(
    conn: &Connection,
    source: &JobRecoverySource,
    canonical_id: i64,
    now_epoch: i64,
) -> Result<()> {
    let Some(incoming_profile) = crate::db::job::dream_profile_key(&source.payload_json) else {
        return Ok(());
    };
    let (state, canonical_payload): (String, String) = conn
        .query_row(
            "SELECT state, payload_json FROM jobs
             WHERE id = ?1 AND job_type = 'dream' AND project = ?2
               AND state IN ('pending', 'processing')",
            params![canonical_id, source.project],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .context("read active Dream recovery canonical")?;
    if state != "pending"
        || crate::db::job::dream_profile_key(&canonical_payload) == Some(incoming_profile)
    {
        return Ok(());
    }
    let changed = conn.execute(
        "UPDATE jobs
         SET host = ?1, payload_json = ?2, priority = min(priority, ?3),
             updated_at_epoch = ?4
         WHERE id = ?5 AND job_type = 'dream' AND project = ?6 AND state = 'pending'",
        params![
            source.host,
            source.payload_json,
            source.priority,
            now_epoch,
            canonical_id,
            source.project
        ],
    )?;
    if changed != 1 {
        bail!("pending Dream recovery canonical changed: canonical_id={canonical_id}");
    }
    Ok(())
}

fn bounded_job_recovery_error(existing_error: Option<&str>, marker: &str) -> String {
    match existing_error.filter(|error| !error.is_empty()) {
        Some(error) => {
            let available = JOB_RECOVERY_ERROR_LIMIT.saturating_sub(marker.len() + 1);
            format!("{} {marker}", crate::db::truncate_str(error, available))
        }
        None => marker.to_string(),
    }
}

fn job_identity_kind(job_type: &str) -> crate::db::JobIdentityKind {
    match job_type {
        "dream" => crate::db::JobIdentityKind::Dream,
        "compile_rules" => crate::db::JobIdentityKind::CompileRules,
        _ => crate::db::JobIdentityKind::Ordinary,
    }
}

fn is_job_identity_conflict(
    error: &SqliteError,
    identity_kind: crate::db::JobIdentityKind,
) -> bool {
    let SqliteError::SqliteFailure(code, message) = error else {
        return false;
    };
    if code.code != ErrorCode::ConstraintViolation {
        return false;
    }
    let message = message.as_deref().unwrap_or_default();
    match identity_kind {
        crate::db::JobIdentityKind::Ordinary => message.contains("idx_jobs_active_ordinary_unique"),
        crate::db::JobIdentityKind::Dream => {
            message.contains("idx_jobs_active_dream_unique")
                || message.contains("UNIQUE constraint failed: jobs.project")
        }
        crate::db::JobIdentityKind::CompileRules => {
            message.contains("idx_jobs_active_compile_rules_unique")
                || message.contains("UNIQUE constraint failed: jobs.project, jobs.state")
        }
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
pub(super) struct JobRecoveryTestSeam {
    pub(super) candidates_collected: Option<std::sync::Arc<std::sync::Barrier>>,
    pub(super) canonical_committed: Option<std::sync::Arc<std::sync::Barrier>>,
    pub(super) skip_initial_lookup: bool,
    pub(super) unreadable_canonical_reread: bool,
}

#[cfg(test)]
thread_local! {
    static JOB_RECOVERY_TEST_SEAM: std::cell::RefCell<JobRecoveryTestSeam> =
        std::cell::RefCell::new(JobRecoveryTestSeam::default());
}

#[cfg(test)]
pub(super) fn set_job_recovery_test_seam(seam: JobRecoveryTestSeam) {
    JOB_RECOVERY_TEST_SEAM.with(|slot| *slot.borrow_mut() = seam);
}

#[cfg(test)]
fn test_after_job_candidates_collected() {
    JOB_RECOVERY_TEST_SEAM.with(|slot| {
        let seam = slot.borrow();
        if let (Some(collected), Some(committed)) =
            (&seam.candidates_collected, &seam.canonical_committed)
        {
            collected.wait();
            committed.wait();
        }
    });
}

#[cfg(not(test))]
fn test_after_job_candidates_collected() {}

#[cfg(test)]
fn test_skip_initial_job_canonical_lookup() -> bool {
    JOB_RECOVERY_TEST_SEAM.with(|slot| slot.borrow().skip_initial_lookup)
}

#[cfg(not(test))]
fn test_skip_initial_job_canonical_lookup() -> bool {
    false
}

#[cfg(test)]
fn test_before_job_canonical_reread() -> Result<()> {
    if JOB_RECOVERY_TEST_SEAM.with(|slot| slot.borrow().unreadable_canonical_reread) {
        bail!("injected unreadable job recovery canonical");
    }
    Ok(())
}

#[cfg(not(test))]
fn test_before_job_canonical_reread() -> Result<()> {
    Ok(())
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
