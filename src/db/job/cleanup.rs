use anyhow::{bail, Context, Result};
use rusqlite::{
    params, Connection, Error as SqliteError, ErrorCode, OptionalExtension, Transaction,
    TransactionBehavior,
};

const CLEANUP_COOLDOWN_SECS: i64 = 24 * 60 * 60;
const CLEANUP_HOST: &str = "remem-worker";
const CLEANUP_PROJECT: &str = "__remem_global_cleanup__";
const CLEANUP_PAYLOAD_JSON: &str = "{}";
const CLEANUP_PRIORITY: i64 = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupEnqueueDecision {
    Enqueued(i64),
    CoalescedInflight(i64),
    SuppressedCooldown(i64),
}

impl CleanupEnqueueDecision {
    pub fn disposition(self) -> &'static str {
        match self {
            Self::Enqueued(_) => "enqueued",
            Self::CoalescedInflight(_) => "coalesced_inflight",
            Self::SuppressedCooldown(_) => "suppressed_cooldown",
        }
    }
}

pub fn maybe_enqueue_cleanup_job(conn: &Connection) -> Result<CleanupEnqueueDecision> {
    maybe_enqueue_cleanup_job_at(conn, chrono::Utc::now().timestamp())
}

pub fn maybe_enqueue_cleanup_job_at(
    conn: &Connection,
    now_epoch: i64,
) -> Result<CleanupEnqueueDecision> {
    if !conn.is_autocommit() {
        bail!("Cleanup scheduling requires an autocommit connection for BEGIN IMMEDIATE");
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .context("begin atomic Cleanup enqueue transaction")?;
    let decision = maybe_enqueue_cleanup_job_core(&tx, now_epoch)?;
    tx.commit()
        .context("commit atomic Cleanup enqueue transaction")?;
    Ok(decision)
}

fn maybe_enqueue_cleanup_job_core(
    conn: &Connection,
    now_epoch: i64,
) -> Result<CleanupEnqueueDecision> {
    if let Some(job_id) = active_cleanup_job(conn)? {
        return Ok(CleanupEnqueueDecision::CoalescedInflight(job_id));
    }

    let cooldown_cutoff = now_epoch.saturating_sub(CLEANUP_COOLDOWN_SECS);
    let recent_run: Option<i64> = conn
        .query_row(
            "SELECT id
             FROM maintenance_runs
             WHERE \"trigger\" = 'automatic'
               AND outcome IN ('success', 'failure')
               AND finished_at_epoch >= ?1
             ORDER BY finished_at_epoch DESC, id DESC
             LIMIT 1",
            params![cooldown_cutoff],
            |row| row.get(0),
        )
        .optional()
        .context("read automatic Cleanup cooldown")?;
    if let Some(run_id) = recent_run {
        return Ok(CleanupEnqueueDecision::SuppressedCooldown(run_id));
    }

    let inserted = conn.execute(
        "INSERT INTO jobs
         (host, job_type, project, session_id, payload_json, state, priority,
          attempt_count, max_attempts, lease_owner, lease_expires_epoch,
          next_retry_epoch, last_error, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'cleanup', ?2, NULL, ?3, 'pending', ?4,
                 0, 6, NULL, NULL, ?5, NULL, ?5, ?5)",
        params![
            CLEANUP_HOST,
            CLEANUP_PROJECT,
            CLEANUP_PAYLOAD_JSON,
            CLEANUP_PRIORITY,
            now_epoch
        ],
    );
    match inserted {
        Ok(1) => Ok(CleanupEnqueueDecision::Enqueued(conn.last_insert_rowid())),
        Ok(count) => bail!("Cleanup enqueue invariant violated: inserted_rows={count}"),
        Err(error) if is_cleanup_identity_conflict(&error) => active_cleanup_job(conn)?
            .map(CleanupEnqueueDecision::CoalescedInflight)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Cleanup identity conflict had no active canonical after insert conflict"
                )
            }),
        Err(error) => Err(error).context("insert global Cleanup job"),
    }
}

fn active_cleanup_job(conn: &Connection) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT id
         FROM jobs
         WHERE job_type = 'cleanup'
           AND state IN ('pending', 'processing')
         ORDER BY CASE state WHEN 'processing' THEN 0 ELSE 1 END, id ASC
         LIMIT 1",
        [],
        |row| row.get(0),
    )
    .optional()
    .context("read active global Cleanup job")
}

fn is_cleanup_identity_conflict(error: &SqliteError) -> bool {
    let SqliteError::SqliteFailure(code, message) = error else {
        return false;
    };
    code.code == ErrorCode::ConstraintViolation
        && message.as_deref().is_some_and(|message| {
            message.contains("idx_jobs_active_cleanup_unique")
                || message.contains("UNIQUE constraint failed: jobs.job_type")
        })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    use anyhow::Result;
    use rusqlite::{params, Connection};

    use super::{
        maybe_enqueue_cleanup_job_at, CleanupEnqueueDecision, CLEANUP_COOLDOWN_SECS, CLEANUP_HOST,
        CLEANUP_PAYLOAD_JSON, CLEANUP_PRIORITY, CLEANUP_PROJECT,
    };
    use crate::db::{
        enqueue_job, maintain_failure_lifecycle, release_expired_job_leases,
        ExpiredJobLeaseOutcome, JobIdentityKind, JobType,
    };
    use crate::migrate::MIGRATIONS;

    fn setup_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        for migration in MIGRATIONS {
            conn.execute_batch(migration.sql)?;
        }
        Ok(conn)
    }

    fn finish_job(conn: &Connection, job_id: i64, now_epoch: i64) -> Result<()> {
        conn.execute(
            "UPDATE jobs
             SET state = 'done', updated_at_epoch = ?1
             WHERE id = ?2",
            params![now_epoch, job_id],
        )?;
        Ok(())
    }

    fn insert_run(
        conn: &Connection,
        trigger: &str,
        outcome: &str,
        finished_at_epoch: i64,
    ) -> Result<i64> {
        let (counts_json, error): (Option<&str>, Option<&str>) = match outcome {
            "success" => (Some(r#"{"expired_memories":0}"#), None),
            "failure" => (None, Some("injected cleanup failure")),
            other => anyhow::bail!("unsupported test outcome: {other}"),
        };
        conn.execute(
            "INSERT INTO maintenance_runs
             (job_id, \"trigger\", policy_version, started_at_epoch,
              finished_at_epoch, outcome, counts_json, error)
             VALUES (NULL, ?1, 1, ?2, ?3, ?4, ?5, ?6)",
            params![
                trigger,
                finished_at_epoch - 1,
                finished_at_epoch,
                outcome,
                counts_json,
                error
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    #[test]
    fn cleanup_enqueue_uses_fixed_global_identity_and_coalesces_active() -> Result<()> {
        let conn = setup_conn()?;
        let now = 1_700_000_000;

        let CleanupEnqueueDecision::Enqueued(job_id) = maybe_enqueue_cleanup_job_at(&conn, now)?
        else {
            anyhow::bail!("first Cleanup decision should enqueue");
        };
        let row: (String, String, Option<String>, String, i64) = conn.query_row(
            "SELECT host, project, session_id, payload_json, priority
             FROM jobs WHERE id = ?1",
            [job_id],
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
        assert_eq!(
            row,
            (
                CLEANUP_HOST.to_string(),
                CLEANUP_PROJECT.to_string(),
                None,
                CLEANUP_PAYLOAD_JSON.to_string(),
                CLEANUP_PRIORITY
            )
        );
        assert_eq!(
            maybe_enqueue_cleanup_job_at(&conn, now + 1)?,
            CleanupEnqueueDecision::CoalescedInflight(job_id)
        );

        let error = enqueue_job(
            &conn,
            "caller",
            JobType::Cleanup,
            "/caller-controlled",
            Some("session"),
            r#"{"purge_archived_failures":true}"#,
            0,
        )
        .expect_err("direct Cleanup enqueue must reject caller policy");
        assert!(error.to_string().contains("must be scheduled"));
        Ok(())
    }

    #[test]
    fn automatic_success_and_failure_suppress_for_inclusive_cooldown() -> Result<()> {
        for outcome in ["success", "failure"] {
            let conn = setup_conn()?;
            let now = 1_700_000_000;
            let CleanupEnqueueDecision::Enqueued(job_id) =
                maybe_enqueue_cleanup_job_at(&conn, now - CLEANUP_COOLDOWN_SECS - 10)?
            else {
                anyhow::bail!("fixture Cleanup decision should enqueue");
            };
            finish_job(&conn, job_id, now - CLEANUP_COOLDOWN_SECS - 9)?;
            let run_id = insert_run(&conn, "automatic", outcome, now - CLEANUP_COOLDOWN_SECS)?;

            assert_eq!(
                maybe_enqueue_cleanup_job_at(&conn, now)?,
                CleanupEnqueueDecision::SuppressedCooldown(run_id),
                "outcome={outcome}"
            );

            conn.execute(
                "UPDATE maintenance_runs
                 SET started_at_epoch = ?1, finished_at_epoch = ?1
                 WHERE id = ?2",
                params![now - CLEANUP_COOLDOWN_SECS - 1, run_id],
            )?;
            assert!(
                matches!(
                    maybe_enqueue_cleanup_job_at(&conn, now)?,
                    CleanupEnqueueDecision::Enqueued(_)
                ),
                "outcome={outcome}"
            );
        }
        Ok(())
    }

    #[test]
    fn active_cleanup_precedes_cooldown_and_manual_runs_do_not_suppress() -> Result<()> {
        let conn = setup_conn()?;
        let now = 1_700_000_000;
        insert_run(&conn, "manual", "success", now)?;
        let CleanupEnqueueDecision::Enqueued(job_id) = maybe_enqueue_cleanup_job_at(&conn, now)?
        else {
            anyhow::bail!("manual maintenance must not suppress automatic scheduling");
        };
        insert_run(&conn, "automatic", "success", now)?;
        assert_eq!(
            maybe_enqueue_cleanup_job_at(&conn, now + 1)?,
            CleanupEnqueueDecision::CoalescedInflight(job_id)
        );
        Ok(())
    }

    #[test]
    fn expired_cleanup_lease_requeues_with_global_identity_kind() -> Result<()> {
        let conn = setup_conn()?;
        let now = chrono::Utc::now().timestamp();
        let CleanupEnqueueDecision::Enqueued(job_id) = maybe_enqueue_cleanup_job_at(&conn, now)?
        else {
            anyhow::bail!("first Cleanup decision should enqueue");
        };
        conn.execute(
            "UPDATE jobs
             SET state = 'processing', lease_owner = 'dead-worker',
                 lease_expires_epoch = ?1
             WHERE id = ?2",
            params![now - 1, job_id],
        )?;

        let batch = release_expired_job_leases(&conn)?;
        assert_eq!(
            batch.outcomes,
            vec![ExpiredJobLeaseOutcome::Requeued {
                source_id: job_id,
                identity_kind: JobIdentityKind::Cleanup,
            }]
        );
        assert_eq!(
            maybe_enqueue_cleanup_job_at(&conn, now + 1)?,
            CleanupEnqueueDecision::CoalescedInflight(job_id)
        );
        Ok(())
    }

    #[test]
    fn failed_cleanup_recovery_coalesces_to_global_active_canonical() -> Result<()> {
        let conn = setup_conn()?;
        let now = chrono::Utc::now().timestamp();
        let CleanupEnqueueDecision::Enqueued(source_id) =
            maybe_enqueue_cleanup_job_at(&conn, now - 2_000)?
        else {
            anyhow::bail!("source Cleanup decision should enqueue");
        };
        conn.execute(
            "UPDATE jobs
             SET state = 'failed', failure_class = 'transient',
                 failed_at_epoch = ?1, updated_at_epoch = ?1,
                 last_error = 'database is locked'
             WHERE id = ?2",
            params![now - 1_000, source_id],
        )?;
        let CleanupEnqueueDecision::Enqueued(canonical_id) =
            maybe_enqueue_cleanup_job_at(&conn, now - 500)?
        else {
            anyhow::bail!("canonical Cleanup decision should enqueue");
        };
        conn.execute(
            "UPDATE jobs
             SET host = 'different-worker', project = '/different-project',
                 session_id = 'different-session'
             WHERE id = ?1",
            [canonical_id],
        )?;

        let result = maintain_failure_lifecycle(&conn)?;
        assert_eq!(result.retried_jobs, 0);
        assert_eq!(result.coalesced_jobs, 1);
        let source: (String, String) = conn.query_row(
            "SELECT state, failure_class FROM jobs WHERE id = ?1",
            [source_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(source, ("failed".to_string(), "permanent".to_string()));
        Ok(())
    }

    #[test]
    fn concurrent_cleanup_probes_coalesce_under_begin_immediate() -> Result<()> {
        let path = crate::db::test_support::unique_temp_db_path("cleanup-enqueue");
        let initial = Connection::open(&path)?;
        initial.pragma_update(None, "journal_mode", "WAL")?;
        initial.busy_timeout(Duration::from_secs(30))?;
        for migration in MIGRATIONS {
            initial.execute_batch(migration.sql)?;
        }
        drop(initial);

        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || -> Result<CleanupEnqueueDecision> {
                let conn = Connection::open(path)?;
                conn.busy_timeout(Duration::from_secs(30))?;
                barrier.wait();
                maybe_enqueue_cleanup_job_at(&conn, 1_700_000_000)
            }));
        }
        barrier.wait();
        let decisions = handles
            .into_iter()
            .map(|handle| handle.join().expect("Cleanup probe thread should join"))
            .collect::<Result<Vec<_>>>()?;

        let enqueued = decisions
            .iter()
            .find_map(|decision| match decision {
                CleanupEnqueueDecision::Enqueued(id) => Some(*id),
                _ => None,
            })
            .expect("one Cleanup probe should enqueue");
        assert_eq!(
            decisions
                .iter()
                .filter(|decision| {
                    **decision == CleanupEnqueueDecision::CoalescedInflight(enqueued)
                })
                .count(),
            1,
            "got: {decisions:?}"
        );
        crate::db::test_support::cleanup_temp_db_files(&path);
        Ok(())
    }
}
