use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::db::job::{Job, JobType};

pub fn claim_next_job(
    conn: &mut Connection,
    lease_owner: &str,
    lease_secs: i64,
) -> Result<Option<Job>> {
    let now = chrono::Utc::now().timestamp();
    let lease_expires = now + lease_secs.max(1);
    let tx = conn.transaction()?;
    let candidate: Option<i64> = tx
        .query_row(
            "SELECT candidate.id FROM jobs AS candidate
             WHERE candidate.state = 'pending'
               AND candidate.job_type <> 'cleanup'
               AND candidate.next_retry_epoch <= ?1
               AND NOT (
                   candidate.job_type = 'compile_rules'
                   AND EXISTS (
                       SELECT 1 FROM jobs AS predecessor
                       WHERE predecessor.job_type = 'compile_rules'
                         AND predecessor.project = candidate.project
                         AND predecessor.state = 'processing'
                   )
               )
             ORDER BY candidate.priority ASC,
                      candidate.created_at_epoch ASC,
                      candidate.id ASC
             LIMIT 1",
            params![now],
            |row| row.get(0),
        )
        .optional()?;

    let Some(job_id) = candidate else {
        tx.commit()?;
        return Ok(None);
    };

    let updated = tx.execute(
        "UPDATE jobs AS candidate
         SET state = 'processing',
             lease_owner = ?1,
             lease_expires_epoch = ?2,
             updated_at_epoch = ?3
         WHERE candidate.id = ?4
           AND candidate.state = 'pending'
           AND candidate.job_type <> 'cleanup'
           AND NOT (
               candidate.job_type = 'compile_rules'
               AND EXISTS (
                   SELECT 1 FROM jobs AS predecessor
                   WHERE predecessor.job_type = 'compile_rules'
                     AND predecessor.project = candidate.project
                     AND predecessor.state = 'processing'
               )
           )",
        params![lease_owner, lease_expires, now, job_id],
    )?;
    if updated == 0 {
        tx.commit()?;
        return Ok(None);
    }

    let job = load_claimed_job(&tx, job_id)?;
    tx.commit()?;
    Ok(Some(job))
}

pub fn claim_ready_cleanup_job(
    conn: &mut Connection,
    lease_owner: &str,
    lease_secs: i64,
) -> Result<Option<Job>> {
    let now = chrono::Utc::now().timestamp();
    let lease_expires = now + lease_secs.max(1);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let candidate: Option<i64> = tx
        .query_row(
            "SELECT id
             FROM jobs
             WHERE job_type = 'cleanup'
               AND state = 'pending'
               AND next_retry_epoch <= ?1
             ORDER BY priority ASC, created_at_epoch ASC, id ASC
             LIMIT 1",
            params![now],
            |row| row.get(0),
        )
        .optional()?;

    let Some(job_id) = candidate else {
        tx.commit()?;
        return Ok(None);
    };

    let updated = tx.execute(
        "UPDATE jobs
         SET state = 'processing',
             lease_owner = ?1,
             lease_expires_epoch = ?2,
             updated_at_epoch = ?3
         WHERE id = ?4
           AND job_type = 'cleanup'
           AND state = 'pending'
           AND next_retry_epoch <= ?3",
        params![lease_owner, lease_expires, now, job_id],
    )?;
    if updated == 0 {
        tx.commit()?;
        return Ok(None);
    }

    let job = load_claimed_job(&tx, job_id)?;
    tx.commit()?;
    Ok(Some(job))
}

fn load_claimed_job(conn: &Connection, job_id: i64) -> Result<Job> {
    let row = conn.query_row(
        "SELECT id, host, job_type, project, session_id, payload_json, attempt_count, max_attempts
         FROM jobs WHERE id = ?1",
        params![job_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        },
    )?;

    Ok(Job {
        id: row.0,
        host: row.1,
        job_type: JobType::from_db(&row.2)?,
        project: row.3,
        session_id: row.4,
        payload_json: row.5,
        attempt_count: row.6,
        max_attempts: row.7,
    })
}

#[cfg(test)]
mod eligibility_tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
        time::Duration,
    };

    use rusqlite::{params, Connection};

    use super::{claim_next_job, claim_ready_cleanup_job};
    use crate::db::{enqueue_job, maybe_enqueue_cleanup_job_at, CleanupEnqueueDecision, JobType};
    use crate::migrate::MIGRATIONS;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        for migration in MIGRATIONS {
            conn.execute_batch(migration.sql)
                .expect("schema migration should load");
        }
        conn
    }

    fn compile_rules_with_successor(conn: &Connection) -> (i64, i64) {
        let source = enqueue_job(
            conn,
            "worker",
            JobType::CompileRules,
            "compile-project",
            None,
            "{}",
            1,
        )
        .expect("CompileRules source should enqueue");
        conn.execute(
            "UPDATE jobs SET state = 'processing', lease_owner = 'worker-a',
                 lease_expires_epoch = ?2 WHERE id = ?1",
            params![source, chrono::Utc::now().timestamp() + 60],
        )
        .expect("CompileRules source should enter processing");
        let successor = enqueue_job(
            conn,
            "worker",
            JobType::CompileRules,
            "compile-project",
            None,
            "{}",
            1,
        )
        .expect("CompileRules successor should enqueue");
        (source, successor)
    }

    #[test]
    fn claim_next_job_skips_compile_rules_successor_while_predecessor_processing() {
        let mut conn = setup_conn();
        let (_, successor) = compile_rules_with_successor(&conn);

        let claimed =
            claim_next_job(&mut conn, "worker-b", 60).expect("claim query should succeed");

        assert!(claimed.is_none());
        let state: String = conn
            .query_row(
                "SELECT state FROM jobs WHERE id = ?1",
                params![successor],
                |row| row.get(0),
            )
            .expect("successor state should load");
        assert_eq!(state, "pending");
    }

    #[test]
    fn claim_next_job_continues_to_unrelated_eligible_job() {
        let mut conn = setup_conn();
        compile_rules_with_successor(&conn);
        let ordinary = enqueue_job(
            &conn,
            "codex-cli",
            JobType::Compress,
            "ordinary-project",
            None,
            "{}",
            2,
        )
        .expect("ordinary job should enqueue");

        let claimed = claim_next_job(&mut conn, "worker-b", 60)
            .expect("claim query should succeed")
            .expect("unrelated job should remain eligible");

        assert_eq!(claimed.id, ordinary);
    }

    #[test]
    fn ordinary_claim_lane_skips_cleanup_jobs() {
        let mut conn = setup_conn();
        let CleanupEnqueueDecision::Enqueued(cleanup_id) =
            maybe_enqueue_cleanup_job_at(&conn, chrono::Utc::now().timestamp())
                .expect("Cleanup should enqueue")
        else {
            panic!("first Cleanup should enqueue");
        };

        assert!(claim_next_job(&mut conn, "ordinary-worker", 60)
            .expect("ordinary claim should succeed")
            .is_none());
        let state: String = conn
            .query_row(
                "SELECT state FROM jobs WHERE id = ?1",
                params![cleanup_id],
                |row| row.get(0),
            )
            .expect("Cleanup state should load");
        assert_eq!(state, "pending");
    }

    #[test]
    fn cleanup_claim_lane_ignores_ordinary_jobs() {
        let mut conn = setup_conn();
        enqueue_job(
            &conn,
            "codex-cli",
            JobType::Compress,
            "ordinary-project",
            None,
            "{}",
            0,
        )
        .expect("ordinary job should enqueue");
        let CleanupEnqueueDecision::Enqueued(cleanup_id) =
            maybe_enqueue_cleanup_job_at(&conn, chrono::Utc::now().timestamp())
                .expect("Cleanup should enqueue")
        else {
            panic!("first Cleanup should enqueue");
        };

        let claimed = claim_ready_cleanup_job(&mut conn, "cleanup-worker", 60)
            .expect("Cleanup claim should succeed")
            .expect("Cleanup should be ready");
        assert_eq!(claimed.id, cleanup_id);
        assert_eq!(claimed.job_type, JobType::Cleanup);
    }

    #[test]
    fn concurrent_cleanup_claims_serialize_without_busy_errors() -> anyhow::Result<()> {
        let path = crate::db::test_support::unique_temp_db_path("cleanup-claim");
        let initial = Connection::open(&path)?;
        initial.pragma_update(None, "journal_mode", "WAL")?;
        initial.busy_timeout(Duration::from_secs(30))?;
        for migration in MIGRATIONS {
            initial.execute_batch(migration.sql)?;
        }
        let CleanupEnqueueDecision::Enqueued(cleanup_id) =
            maybe_enqueue_cleanup_job_at(&initial, chrono::Utc::now().timestamp())?
        else {
            anyhow::bail!("Cleanup should enqueue before concurrent claims");
        };
        drop(initial);

        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for worker in ["cleanup-a", "cleanup-b"] {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(
                move || -> anyhow::Result<Option<crate::db::Job>> {
                    let mut conn = Connection::open(path)?;
                    conn.busy_timeout(Duration::from_secs(30))?;
                    barrier.wait();
                    claim_ready_cleanup_job(&mut conn, worker, 60)
                },
            ));
        }
        barrier.wait();
        let claims = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("Cleanup claim thread panicked"))?
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
        assert_eq!(claims.iter().filter(|claim| claim.is_none()).count(), 1);
        assert_eq!(
            claims
                .iter()
                .filter_map(|claim| claim.as_ref().map(|job| job.id))
                .collect::<Vec<_>>(),
            vec![cleanup_id]
        );
        crate::db::test_support::cleanup_temp_db_files(&path);
        Ok(())
    }
}
