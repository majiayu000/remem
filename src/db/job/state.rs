use anyhow::Result;
use rusqlite::{params, Connection};

pub fn mark_job_done(conn: &Connection, job_id: i64, lease_owner: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE jobs
         SET state = 'done',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             updated_at_epoch = ?1
         WHERE id = ?2 AND lease_owner = ?3",
        params![now, job_id, lease_owner],
    )?;
    Ok(())
}

pub fn mark_job_failed(
    conn: &Connection,
    job_id: i64,
    lease_owner: &str,
    error_msg: &str,
    retry_delay_secs: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let next_retry = now + retry_delay_secs;
    conn.execute(
        "UPDATE jobs
         SET state = 'pending',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             attempt_count = attempt_count + 1,
             next_retry_epoch = ?1,
             last_error = ?2,
             updated_at_epoch = ?3
         WHERE id = ?4 AND lease_owner = ?5",
        params![next_retry, error_msg, now, job_id, lease_owner],
    )?;
    Ok(())
}

pub fn mark_job_exhausted(conn: &Connection, job_id: i64, lease_owner: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE jobs
         SET state = 'failed',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             updated_at_epoch = ?1
         WHERE id = ?2 AND lease_owner = ?3",
        params![now, job_id, lease_owner],
    )?;
    Ok(())
}

pub fn release_expired_job_leases(conn: &Connection) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let count = conn.execute(
        "UPDATE jobs
         SET state = 'pending',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             updated_at_epoch = ?1
         WHERE state = 'processing'
           AND lease_expires_epoch IS NOT NULL
           AND lease_expires_epoch < ?1",
        params![now],
    )?;
    Ok(count)
}

pub fn requeue_stuck_jobs(conn: &Connection) -> Result<usize> {
    release_expired_job_leases(conn)
}

pub fn mark_job_failed_or_retry(
    conn: &Connection,
    job_id: i64,
    lease_owner: &str,
    err: &str,
    backoff_secs: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let (attempt_count, max_attempts): (i64, i64) = conn.query_row(
        "SELECT attempt_count, max_attempts FROM jobs WHERE id = ?1",
        params![job_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let next_attempt = attempt_count + 1;
    if next_attempt >= max_attempts {
        conn.execute(
            "UPDATE jobs
             SET state = 'failed',
                 attempt_count = ?1,
                 last_error = ?2,
                 lease_owner = NULL,
                 lease_expires_epoch = NULL,
                 updated_at_epoch = ?3
             WHERE id = ?4 AND lease_owner = ?5",
            params![
                next_attempt,
                crate::db::truncate_str(err, 2000),
                now,
                job_id,
                lease_owner
            ],
        )?;
        return Ok(());
    }

    conn.execute(
        "UPDATE jobs
         SET state = 'pending',
             attempt_count = ?1,
             next_retry_epoch = ?2,
             last_error = ?3,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             updated_at_epoch = ?4
         WHERE id = ?5 AND lease_owner = ?6",
        params![
            next_attempt,
            now + backoff_secs.max(1),
            crate::db::truncate_str(err, 2000),
            now,
            job_id,
            lease_owner
        ],
    )?;
    Ok(())
}
