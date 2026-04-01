use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::db_job::{Job, JobType};

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
            "SELECT id FROM jobs
             WHERE state = 'pending' AND next_retry_epoch <= ?1
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
         WHERE id = ?4 AND state = 'pending'",
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
        "SELECT id, job_type, project, session_id, payload_json, attempt_count, max_attempts
         FROM jobs WHERE id = ?1",
        params![job_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        },
    )?;

    Ok(Job {
        id: row.0,
        job_type: JobType::from_db(&row.1)?,
        project: row.2,
        session_id: row.3,
        payload_json: row.4,
        attempt_count: row.5,
        max_attempts: row.6,
    })
}
