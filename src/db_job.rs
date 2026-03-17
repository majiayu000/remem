use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

pub use crate::db_models::{Job, JobType};

pub fn enqueue_job(
    conn: &Connection,
    job_type: JobType,
    project: &str,
    session_id: Option<&str>,
    payload_json: &str,
    priority: i64,
) -> Result<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM jobs
             WHERE job_type = ?1
               AND project = ?2
               AND COALESCE(session_id, '') = COALESCE(?3, '')
               AND state IN ('pending', 'processing')
             ORDER BY id DESC
             LIMIT 1",
            params![job_type.as_str(), project, session_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }

    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO jobs
         (job_type, project, session_id, payload_json, state, priority,
          attempt_count, max_attempts, lease_owner, lease_expires_epoch,
          next_retry_epoch, last_error, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, 'pending', ?5, 0, 6, NULL, NULL, ?6, NULL, ?6, ?6)",
        params![job_type.as_str(), project, session_id, payload_json, priority, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn claim_next_job(conn: &mut Connection, lease_owner: &str, lease_secs: i64) -> Result<Option<Job>> {
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

    let row = tx.query_row(
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
    tx.commit()?;
    Ok(Some(Job {
        id: row.0,
        job_type: JobType::from_db(&row.1)?,
        project: row.2,
        session_id: row.3,
        payload_json: row.4,
        attempt_count: row.5,
        max_attempts: row.6,
    }))
}

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
            params![next_attempt, crate::db::truncate_str(err, 2000), now, job_id, lease_owner],
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
