use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::job::JobType;

pub fn enqueue_job(
    conn: &Connection,
    host: &str,
    job_type: JobType,
    project: &str,
    session_id: Option<&str>,
    payload_json: &str,
    priority: i64,
) -> Result<i64> {
    // A compile job that is already processing may have read canonical state
    // before the lifecycle mutation which triggered this enqueue. Keep one
    // pending successor instead of deduplicating against that processing row,
    // otherwise the update can be lost when the old worker marks itself done.
    let dedup_processing = i64::from(job_type != JobType::CompileRules);
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM jobs
             WHERE host = ?1
               AND job_type = ?2
               AND project = ?3
               AND COALESCE(session_id, '') = COALESCE(?4, '')
               AND (state = 'pending' OR (?5 = 1 AND state = 'processing'))
             ORDER BY CASE state WHEN 'pending' THEN 0 ELSE 1 END, id DESC
             LIMIT 1",
            params![
                host,
                job_type.as_str(),
                project,
                session_id,
                dedup_processing
            ],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }

    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO jobs
         (host, job_type, project, session_id, payload_json, state, priority,
          attempt_count, max_attempts, lease_owner, lease_expires_epoch,
          next_retry_epoch, last_error, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, 0, 6, NULL, NULL, ?7, NULL, ?7, ?7)",
        params![
            host,
            job_type.as_str(),
            project,
            session_id,
            payload_json,
            priority,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn maybe_enqueue_dream_job(
    conn: &Connection,
    host: &str,
    project: &str,
    payload_json: &str,
    priority: i64,
    cooldown_secs: i64,
) -> Result<Option<i64>> {
    let incoming_profile = dream_profile_key(payload_json);
    let inflight: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT id, state, payload_json FROM jobs
             WHERE job_type = ?1
               AND project = ?2
               AND session_id IS NULL
               AND state IN ('pending', 'processing')
             ORDER BY CASE state WHEN 'pending' THEN 0 ELSE 1 END,
                      updated_at_epoch DESC,
                      id DESC
             LIMIT 1",
            params![JobType::Dream.as_str(), project],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some((id, state, existing_payload)) = inflight {
        if state == "pending"
            && incoming_profile.is_some()
            && dream_profile_key(&existing_payload) != incoming_profile
        {
            let now = chrono::Utc::now().timestamp();
            conn.execute(
                "UPDATE jobs
                 SET host = ?1,
                     payload_json = ?2,
                     priority = CASE WHEN priority <= ?3 THEN priority ELSE ?3 END,
                     updated_at_epoch = ?4
                 WHERE id = ?5 AND state = 'pending'",
                params![host, payload_json, priority, now, id],
            )?;
        }
        return Ok(None);
    }

    let now = chrono::Utc::now().timestamp();
    let cutoff = now - cooldown_secs.max(1);
    let recent_done: Option<i64> = conn
        .query_row(
            "SELECT id FROM jobs
             WHERE job_type = ?1
               AND project = ?2
               AND session_id IS NULL
               AND state = 'done'
               AND updated_at_epoch >= ?3
             ORDER BY updated_at_epoch DESC, id DESC
             LIMIT 1",
            params![JobType::Dream.as_str(), project, cutoff],
            |row| row.get(0),
        )
        .optional()?;
    if recent_done.is_some() {
        return Ok(None);
    }

    enqueue_job(
        conn,
        host,
        JobType::Dream,
        project,
        None,
        payload_json,
        priority,
    )
    .map(Some)
}

fn dream_profile_key(payload_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(payload_json)
        .ok()
        .and_then(|value| {
            value
                .get(crate::runtime_config::MEMORY_AI_PROFILE_FIELD)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|profile| !profile.is_empty())
                .map(str::to_string)
        })
}
