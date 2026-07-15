use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

const EXPIRED_RELEASE_BATCH_LIMIT: i64 = 100;
const COALESCED_ERROR_LIMIT: usize = 2000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobIdentityKind {
    Ordinary,
    Dream,
    CompileRules,
}

impl JobIdentityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ordinary => "ordinary",
            Self::Dream => "dream",
            Self::CompileRules => "compile_rules",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobTransitionOutcome {
    Transitioned,
    Coalesced {
        source_id: i64,
        canonical_id: i64,
        identity_kind: JobIdentityKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExpiredJobLeaseOutcome {
    Requeued {
        source_id: i64,
        identity_kind: JobIdentityKind,
    },
    Coalesced {
        source_id: i64,
        canonical_id: i64,
        identity_kind: JobIdentityKind,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExpiredJobLeaseBatch {
    pub outcomes: Vec<ExpiredJobLeaseOutcome>,
    pub requeued: usize,
    pub coalesced: usize,
}

#[derive(Debug)]
struct LeaseSnapshot {
    state: String,
    owner: Option<String>,
    expires_epoch: Option<i64>,
}

#[derive(Debug)]
struct RetrySnapshot {
    lease: LeaseSnapshot,
    job_type: String,
    project: String,
    priority: i64,
    attempt_count: i64,
    max_attempts: i64,
}

#[derive(Debug)]
struct ExpiredSnapshot {
    state: String,
    job_type: String,
    project: String,
    priority: i64,
    lease_expires_epoch: Option<i64>,
    last_error: Option<String>,
}

pub fn mark_job_done(conn: &Connection, job_id: i64, lease_owner: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let tx = immediate_transaction(conn, "mark job done")?;
    let updated = tx.execute(
        "UPDATE jobs
         SET state = 'done',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             failure_class = NULL,
             failed_at_epoch = NULL,
             archived_at_epoch = NULL,
             updated_at_epoch = ?1
         WHERE id = ?2
           AND state = 'processing'
           AND lease_owner = ?3
           AND lease_expires_epoch IS NOT NULL
           AND lease_expires_epoch >= ?1",
        params![now, job_id, lease_owner],
    )?;
    ensure_lease_transition(&tx, updated, job_id, lease_owner)?;
    tx.commit().context("commit mark job done transaction")?;
    Ok(())
}

pub fn mark_job_failed(
    conn: &Connection,
    job_id: i64,
    lease_owner: &str,
    error_msg: &str,
    retry_delay_secs: i64,
) -> Result<JobTransitionOutcome> {
    transition_failed_or_retry(
        conn,
        job_id,
        lease_owner,
        error_msg,
        retry_delay_secs,
        true,
        false,
    )
}

pub fn mark_job_exhausted(conn: &Connection, job_id: i64, lease_owner: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let tx = immediate_transaction(conn, "mark job exhausted")?;
    let updated = tx.execute(
        "UPDATE jobs
         SET state = 'failed',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             failure_class = COALESCE(failure_class, 'transient'),
             failed_at_epoch = COALESCE(failed_at_epoch, ?1),
             archived_at_epoch = NULL,
             updated_at_epoch = ?1
         WHERE id = ?2
           AND state = 'processing'
           AND lease_owner = ?3
           AND lease_expires_epoch IS NOT NULL
           AND lease_expires_epoch >= ?1",
        params![now, job_id, lease_owner],
    )?;
    ensure_lease_transition(&tx, updated, job_id, lease_owner)?;
    tx.commit()
        .context("commit mark job exhausted transaction")?;
    Ok(())
}

pub fn release_expired_job_leases(conn: &Connection) -> Result<ExpiredJobLeaseBatch> {
    let now = chrono::Utc::now().timestamp();
    let candidates = {
        let mut statement = conn.prepare(
            "SELECT id FROM jobs
             WHERE state = 'processing'
               AND lease_expires_epoch IS NOT NULL
               AND lease_expires_epoch < ?1
             ORDER BY lease_expires_epoch ASC, id ASC
             LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![now, EXPIRED_RELEASE_BATCH_LIMIT], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        rows
    };

    let mut batch = ExpiredJobLeaseBatch::default();
    for source_id in candidates {
        let Some(outcome) = release_expired_job_lease(conn, source_id, now)
            .with_context(|| format!("release expired job lease source_id={source_id}"))?
        else {
            continue;
        };
        match outcome {
            ExpiredJobLeaseOutcome::Requeued { .. } => batch.requeued += 1,
            ExpiredJobLeaseOutcome::Coalesced { .. } => batch.coalesced += 1,
        }
        batch.outcomes.push(outcome);
    }
    Ok(batch)
}

pub fn requeue_stuck_jobs(conn: &Connection) -> Result<usize> {
    release_expired_job_leases(conn).map(|batch| batch.outcomes.len())
}

pub fn mark_job_failed_or_retry(
    conn: &Connection,
    job_id: i64,
    lease_owner: &str,
    err: &str,
    backoff_secs: i64,
) -> Result<JobTransitionOutcome> {
    transition_failed_or_retry(conn, job_id, lease_owner, err, backoff_secs, false, true)
}

fn transition_failed_or_retry(
    conn: &Connection,
    job_id: i64,
    lease_owner: &str,
    err: &str,
    backoff_secs: i64,
    force_retry: bool,
    clamp_backoff: bool,
) -> Result<JobTransitionOutcome> {
    let now = chrono::Utc::now().timestamp();
    let tx = immediate_transaction(conn, "transition failed job")?;
    let snapshot = load_retry_snapshot(&tx, job_id, lease_owner)?;
    validate_current_lease(job_id, lease_owner, now, &snapshot.lease)?;

    let next_attempt = snapshot.attempt_count + 1;
    let failure_class = crate::db::classify_failure(err);
    if !force_retry
        && (failure_class == crate::db::FailureClass::Permanent
            || next_attempt >= snapshot.max_attempts)
    {
        let updated = tx.execute(
            "UPDATE jobs
             SET state = 'failed', attempt_count = ?1, next_retry_epoch = 0,
                 last_error = ?2, failure_class = ?3,
                 failed_at_epoch = COALESCE(failed_at_epoch, ?4),
                 archived_at_epoch = NULL, lease_owner = NULL,
                 lease_expires_epoch = NULL, updated_at_epoch = ?4
             WHERE id = ?5 AND state = 'processing' AND lease_owner = ?6
               AND lease_expires_epoch IS NOT NULL
               AND lease_expires_epoch >= ?4",
            params![
                next_attempt,
                crate::db::truncate_str(err, COALESCED_ERROR_LIMIT),
                failure_class.as_str(),
                now,
                job_id,
                lease_owner
            ],
        )?;
        ensure_lease_transition(&tx, updated, job_id, lease_owner)?;
        tx.commit().context("commit terminal job failure")?;
        return Ok(JobTransitionOutcome::Transitioned);
    }

    let next_retry_epoch = now
        + if clamp_backoff {
            backoff_secs.max(1)
        } else {
            backoff_secs
        };
    if snapshot.job_type == JobIdentityKind::CompileRules.as_str() {
        if let Some(canonical_id) = pending_compile_rules_successor(&tx, &snapshot.project, job_id)?
        {
            coalesce_compile_rules_retry(
                &tx,
                &snapshot,
                job_id,
                lease_owner,
                canonical_id,
                next_attempt,
                next_retry_epoch,
                err,
                now,
            )?;
            tx.commit().context("commit coalesced CompileRules retry")?;
            return Ok(JobTransitionOutcome::Coalesced {
                source_id: job_id,
                canonical_id,
                identity_kind: JobIdentityKind::CompileRules,
            });
        }
    }

    let updated = tx.execute(
        "UPDATE jobs
         SET state = 'pending', attempt_count = ?1, next_retry_epoch = ?2,
             last_error = ?3, failure_class = NULL, failed_at_epoch = NULL,
             archived_at_epoch = NULL, lease_owner = NULL,
             lease_expires_epoch = NULL, updated_at_epoch = ?4
         WHERE id = ?5 AND state = 'processing' AND lease_owner = ?6
           AND lease_expires_epoch IS NOT NULL
           AND lease_expires_epoch >= ?4",
        params![
            next_attempt,
            next_retry_epoch,
            crate::db::truncate_str(err, COALESCED_ERROR_LIMIT),
            now,
            job_id,
            lease_owner
        ],
    )?;
    ensure_lease_transition(&tx, updated, job_id, lease_owner)?;
    tx.commit().context("commit job retry")?;
    Ok(JobTransitionOutcome::Transitioned)
}

fn immediate_transaction<'a>(conn: &'a Connection, action: &str) -> Result<Transaction<'a>> {
    Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .with_context(|| format!("begin {action} transaction"))
}

fn load_lease_snapshot(tx: &Transaction<'_>, job_id: i64) -> Result<Option<LeaseSnapshot>> {
    tx.query_row(
        "SELECT state, lease_owner, lease_expires_epoch FROM jobs WHERE id = ?1",
        params![job_id],
        |row| {
            Ok(LeaseSnapshot {
                state: row.get(0)?,
                owner: row.get(1)?,
                expires_epoch: row.get(2)?,
            })
        },
    )
    .optional()
    .context("read current job lease snapshot")
}

fn load_retry_snapshot(
    tx: &Transaction<'_>,
    job_id: i64,
    expected_owner: &str,
) -> Result<RetrySnapshot> {
    let snapshot = tx
        .query_row(
            "SELECT state, lease_owner, lease_expires_epoch, job_type, project,
                    priority, attempt_count, max_attempts
             FROM jobs WHERE id = ?1",
            params![job_id],
            |row| {
                Ok(RetrySnapshot {
                    lease: LeaseSnapshot {
                        state: row.get(0)?,
                        owner: row.get(1)?,
                        expires_epoch: row.get(2)?,
                    },
                    job_type: row.get(3)?,
                    project: row.get(4)?,
                    priority: row.get(5)?,
                    attempt_count: row.get(6)?,
                    max_attempts: row.get(7)?,
                })
            },
        )
        .optional()
        .context("read retry job lease snapshot")?;
    snapshot.ok_or_else(|| transition_error(job_id, expected_owner, None))
}

fn ensure_lease_transition(
    tx: &Transaction<'_>,
    updated: usize,
    job_id: i64,
    expected_owner: &str,
) -> Result<()> {
    match updated {
        1 => Ok(()),
        0 => {
            let current = load_lease_snapshot(tx, job_id)?;
            Err(transition_error(job_id, expected_owner, current.as_ref()))
        }
        count => bail!(
            "job lease transition invariant violated: job_id={job_id} expected_owner={expected_owner} affected_rows={count}"
        ),
    }
}

fn validate_current_lease(
    job_id: i64,
    expected_owner: &str,
    now: i64,
    snapshot: &LeaseSnapshot,
) -> Result<()> {
    if snapshot.state == "processing"
        && snapshot.owner.as_deref() == Some(expected_owner)
        && snapshot.expires_epoch.is_some_and(|expiry| expiry >= now)
    {
        return Ok(());
    }
    Err(transition_error(job_id, expected_owner, Some(snapshot)))
}

fn transition_error(
    job_id: i64,
    expected_owner: &str,
    snapshot: Option<&LeaseSnapshot>,
) -> anyhow::Error {
    let Some(snapshot) = snapshot else {
        return anyhow::anyhow!(
            "job lease transition rejected: job_id={job_id} expected_owner={expected_owner} current=missing"
        );
    };
    let owner = snapshot.owner.as_deref().unwrap_or("<null>");
    let expiry = snapshot
        .expires_epoch
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<null>".to_string());
    anyhow::anyhow!(
        "job lease transition rejected: job_id={job_id} expected_owner={expected_owner} current_state={} current_owner={owner} lease_expires_epoch={expiry}",
        snapshot.state
    )
}

fn pending_compile_rules_successor(
    tx: &Transaction<'_>,
    project: &str,
    source_id: i64,
) -> Result<Option<i64>> {
    tx.query_row(
        "SELECT id FROM jobs
         WHERE job_type = 'compile_rules' AND project = ?1
           AND state = 'pending' AND id <> ?2
         ORDER BY id ASC LIMIT 1",
        params![project, source_id],
        |row| row.get(0),
    )
    .optional()
    .context("read pending CompileRules successor")
}

#[allow(clippy::too_many_arguments)]
fn coalesce_compile_rules_retry(
    tx: &Transaction<'_>,
    snapshot: &RetrySnapshot,
    source_id: i64,
    lease_owner: &str,
    canonical_id: i64,
    next_attempt: i64,
    next_retry_epoch: i64,
    error: &str,
    now: i64,
) -> Result<()> {
    let successor_updated = tx.execute(
        "UPDATE jobs
         SET next_retry_epoch = max(next_retry_epoch, ?1),
             priority = min(priority, ?2), updated_at_epoch = ?3
         WHERE id = ?4 AND job_type = 'compile_rules'
           AND project = ?5 AND state = 'pending'",
        params![
            next_retry_epoch,
            snapshot.priority,
            now,
            canonical_id,
            snapshot.project
        ],
    )?;
    if successor_updated != 1 {
        bail!(
            "CompileRules retry canonical changed: source_id={source_id} canonical_id={canonical_id} affected_rows={successor_updated}"
        );
    }

    let last_error = bounded_coalesced_error(error, canonical_id);
    let source_updated = tx.execute(
        "UPDATE jobs
         SET state = 'failed', attempt_count = ?1, next_retry_epoch = 0,
             last_error = ?2, failure_class = 'permanent',
             failed_at_epoch = COALESCE(failed_at_epoch, ?3),
             archived_at_epoch = NULL, lease_owner = NULL,
             lease_expires_epoch = NULL, updated_at_epoch = ?3
         WHERE id = ?4 AND state = 'processing' AND lease_owner = ?5
           AND lease_expires_epoch IS NOT NULL
           AND lease_expires_epoch >= ?3",
        params![next_attempt, last_error, now, source_id, lease_owner],
    )?;
    ensure_lease_transition(tx, source_updated, source_id, lease_owner)
}

fn bounded_coalesced_error(error: &str, canonical_id: i64) -> String {
    let marker = format!("[compile_rules_retry_coalesced_to_successor id={canonical_id}]");
    let primary = if error.is_empty() {
        "expired lease"
    } else {
        error
    };
    let available = COALESCED_ERROR_LIMIT.saturating_sub(marker.len() + 1);
    format!("{} {marker}", crate::db::truncate_str(primary, available))
}

fn identity_kind(job_type: &str) -> JobIdentityKind {
    match job_type {
        "dream" => JobIdentityKind::Dream,
        "compile_rules" => JobIdentityKind::CompileRules,
        _ => JobIdentityKind::Ordinary,
    }
}

fn release_expired_job_lease(
    conn: &Connection,
    source_id: i64,
    now: i64,
) -> Result<Option<ExpiredJobLeaseOutcome>> {
    let tx = immediate_transaction(conn, "release expired job lease")?;
    let source = tx
        .query_row(
            "SELECT state, job_type, project, priority, lease_expires_epoch,
                    last_error
             FROM jobs WHERE id = ?1",
            params![source_id],
            |row| {
                Ok(ExpiredSnapshot {
                    state: row.get(0)?,
                    job_type: row.get(1)?,
                    project: row.get(2)?,
                    priority: row.get(3)?,
                    lease_expires_epoch: row.get(4)?,
                    last_error: row.get(5)?,
                })
            },
        )
        .optional()
        .context("read expired job candidate")?;
    let Some(source) = source else {
        tx.commit().context("commit missing expired job skip")?;
        return Ok(None);
    };
    if source.state != "processing"
        || source
            .lease_expires_epoch
            .is_none_or(|expiry| expiry >= now)
    {
        tx.commit().context("commit ineligible expired job skip")?;
        return Ok(None);
    }

    let kind = identity_kind(&source.job_type);
    if kind == JobIdentityKind::CompileRules {
        if let Some(canonical_id) =
            pending_compile_rules_successor(&tx, &source.project, source_id)?
        {
            let successor_updated = tx.execute(
                "UPDATE jobs
                 SET next_retry_epoch = max(next_retry_epoch, ?1),
                     priority = min(priority, ?2), updated_at_epoch = ?1
                 WHERE id = ?3 AND job_type = 'compile_rules'
                   AND project = ?4 AND state = 'pending'",
                params![now, source.priority, canonical_id, source.project],
            )?;
            if successor_updated != 1 {
                bail!(
                    "expired CompileRules canonical changed: source_id={source_id} canonical_id={canonical_id} affected_rows={successor_updated}"
                );
            }
            let evidence = source.last_error.as_deref().unwrap_or("expired lease");
            let last_error = bounded_coalesced_error(evidence, canonical_id);
            let source_updated = tx.execute(
                "UPDATE jobs
                 SET state = 'failed', next_retry_epoch = 0, last_error = ?1,
                     failure_class = 'permanent',
                     failed_at_epoch = COALESCE(failed_at_epoch, ?2),
                     archived_at_epoch = NULL, lease_owner = NULL,
                     lease_expires_epoch = NULL, updated_at_epoch = ?2
                 WHERE id = ?3 AND state = 'processing'
                   AND lease_expires_epoch IS NOT NULL
                   AND lease_expires_epoch < ?2",
                params![last_error, now, source_id],
            )?;
            if source_updated != 1 {
                bail!(
                    "expired CompileRules source changed: source_id={source_id} affected_rows={source_updated}"
                );
            }
            tx.commit()
                .context("commit coalesced expired CompileRules")?;
            return Ok(Some(ExpiredJobLeaseOutcome::Coalesced {
                source_id,
                canonical_id,
                identity_kind: kind,
            }));
        }
    }

    let updated = tx.execute(
        "UPDATE jobs
         SET state = 'pending', lease_owner = NULL, lease_expires_epoch = NULL,
             updated_at_epoch = ?1
         WHERE id = ?2 AND state = 'processing'
           AND lease_expires_epoch IS NOT NULL
           AND lease_expires_epoch < ?1",
        params![now, source_id],
    )?;
    if updated != 1 {
        bail!("expired job source changed: source_id={source_id} affected_rows={updated}");
    }
    tx.commit().context("commit expired job requeue")?;
    Ok(Some(ExpiredJobLeaseOutcome::Requeued {
        source_id,
        identity_kind: kind,
    }))
}
