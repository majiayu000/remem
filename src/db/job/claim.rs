use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

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
    use rusqlite::{params, Connection};

    use super::claim_next_job;
    use crate::db::{enqueue_job, JobType};
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
}
