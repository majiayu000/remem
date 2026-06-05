use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::ExtractionTaskKind;

pub const EXTRACTION_TASK_MAX_ATTEMPTS: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionTask {
    pub id: i64,
    pub task_kind: ExtractionTaskKind,
    pub host_id: i64,
    pub workspace_id: i64,
    pub project_id: i64,
    pub session_row_id: Option<i64>,
    pub host: String,
    pub project: String,
    pub session_id: Option<String>,
    pub ai_profile: Option<String>,
    pub priority: i64,
    pub cursor_event_id: Option<i64>,
    pub high_watermark_event_id: Option<i64>,
    pub attempts: i64,
}

pub fn enqueue_followup_extraction_task(
    conn: &Connection,
    source: &ExtractionTask,
    task_kind: ExtractionTaskKind,
    high_watermark_event_id: i64,
) -> Result<i64> {
    let session_row_id = source
        .session_row_id
        .ok_or_else(|| anyhow::anyhow!("follow-up extraction task requires session_row_id"))?;
    let now = chrono::Utc::now().timestamp();
    let idempotency_key = format!(
        "{}:{}:{}:{}",
        source.host_id,
        source.project_id,
        session_row_id,
        task_kind.as_str()
    );
    conn.execute(
        "INSERT INTO extraction_tasks
         (task_kind, host_id, workspace_id, project_id, session_row_id, priority, status,
          idempotency_key, cursor_event_id, high_watermark_event_id, attempts,
          next_retry_epoch, lease_owner, lease_expires_epoch, last_error, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, NULL, ?8, 0, NULL, NULL, NULL, NULL, ?9, ?9)
         ON CONFLICT(idempotency_key) DO UPDATE SET
             high_watermark_event_id = MAX(COALESCE(extraction_tasks.high_watermark_event_id, 0), excluded.high_watermark_event_id),
             status = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN 'pending'
                 ELSE extraction_tasks.status
             END,
             next_retry_epoch = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.next_retry_epoch
             END,
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            task_kind.as_str(),
            source.host_id,
            source.workspace_id,
            source.project_id,
            session_row_id,
            task_kind.priority(),
            idempotency_key,
            high_watermark_event_id,
            now
        ],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM extraction_tasks WHERE idempotency_key = ?1",
        params![idempotency_key],
        |row| row.get(0),
    )?)
}

pub fn claim_next_extraction_task(
    conn: &mut Connection,
    lease_owner: &str,
    lease_secs: i64,
) -> Result<Option<ExtractionTask>> {
    let now = chrono::Utc::now().timestamp();
    let lease_expires = now + lease_secs.max(1);
    let tx = conn.transaction()?;
    let candidate: Option<i64> = tx
        .query_row(
            "SELECT id FROM extraction_tasks
             WHERE status = 'pending'
               AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?1)
             ORDER BY priority ASC, created_at_epoch ASC, id ASC
             LIMIT 1",
            params![now],
            |row| row.get(0),
        )
        .optional()?;

    let Some(task_id) = candidate else {
        tx.commit()?;
        return Ok(None);
    };

    let updated = tx.execute(
        "UPDATE extraction_tasks
         SET status = 'processing',
             lease_owner = ?1,
             lease_expires_epoch = ?2,
             updated_at_epoch = ?3
         WHERE id = ?4 AND status = 'pending'",
        params![lease_owner, lease_expires, now, task_id],
    )?;
    if updated == 0 {
        tx.commit()?;
        return Ok(None);
    }

    let task = load_claimed_extraction_task(&tx, task_id)?;
    tx.commit()?;
    Ok(Some(task))
}

pub fn release_expired_extraction_task_leases(conn: &Connection) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let count = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'pending',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             updated_at_epoch = ?1
         WHERE status = 'processing'
           AND lease_expires_epoch IS NOT NULL
           AND lease_expires_epoch < ?1",
        params![now],
    )?;
    Ok(count)
}

pub fn mark_extraction_task_done(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    completed_high_watermark_event_id: Option<i64>,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let updated = conn.execute(
        "UPDATE extraction_tasks
         SET status = CASE
                 WHEN ?4 IS NOT NULL
                  AND high_watermark_event_id IS NOT NULL
                  AND high_watermark_event_id > ?4 THEN 'pending'
                 ELSE 'done'
             END,
             cursor_event_id = ?4,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             next_retry_epoch = NULL,
             last_error = NULL,
             updated_at_epoch = ?1
         WHERE id = ?2 AND lease_owner = ?3 AND status = 'processing'",
        params![now, task_id, lease_owner, completed_high_watermark_event_id],
    )?;
    ensure_task_updated(updated, task_id)
}

pub fn mark_extraction_task_failed(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    err: &str,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let updated = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'failed',
             attempts = attempts + 1,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             next_retry_epoch = NULL,
             last_error = ?1,
             updated_at_epoch = ?2
         WHERE id = ?3 AND lease_owner = ?4 AND status = 'processing'",
        params![
            crate::db::truncate_str(err, 2000),
            now,
            task_id,
            lease_owner
        ],
    )?;
    ensure_task_updated(updated, task_id)
}

pub fn defer_extraction_task(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    reason: &str,
    backoff_secs: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let attempts: i64 = conn.query_row(
        "SELECT attempts FROM extraction_tasks WHERE id = ?1",
        params![task_id],
        |row| row.get(0),
    )?;
    let next_attempt = attempts + 1;
    if next_attempt >= EXTRACTION_TASK_MAX_ATTEMPTS {
        let updated = conn.execute(
            "UPDATE extraction_tasks
             SET status = 'failed',
                 attempts = ?1,
                 lease_owner = NULL,
                 lease_expires_epoch = NULL,
                 next_retry_epoch = NULL,
                 last_error = ?2,
                 updated_at_epoch = ?3
             WHERE id = ?4 AND lease_owner = ?5 AND status = 'processing'",
            params![
                next_attempt,
                crate::db::truncate_str(reason, 2000),
                now,
                task_id,
                lease_owner
            ],
        )?;
        return ensure_task_updated(updated, task_id);
    }

    let updated = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'pending',
             attempts = ?1,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             next_retry_epoch = ?2,
             last_error = ?3,
             updated_at_epoch = ?4
         WHERE id = ?5 AND lease_owner = ?6 AND status = 'processing'",
        params![
            next_attempt,
            now + backoff_secs.max(1),
            crate::db::truncate_str(reason, 2000),
            now,
            task_id,
            lease_owner
        ],
    )?;
    ensure_task_updated(updated, task_id)
}

pub fn mark_extraction_task_failed_or_retry(
    conn: &Connection,
    task_id: i64,
    lease_owner: &str,
    err: &str,
    backoff_secs: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let attempts: i64 = conn.query_row(
        "SELECT attempts FROM extraction_tasks WHERE id = ?1",
        params![task_id],
        |row| row.get(0),
    )?;
    let next_attempt = attempts + 1;
    if next_attempt >= EXTRACTION_TASK_MAX_ATTEMPTS {
        let updated = conn.execute(
            "UPDATE extraction_tasks
             SET status = 'failed',
                 attempts = ?1,
                 lease_owner = NULL,
                 lease_expires_epoch = NULL,
                 next_retry_epoch = NULL,
                 last_error = ?2,
                 updated_at_epoch = ?3
             WHERE id = ?4 AND lease_owner = ?5 AND status = 'processing'",
            params![
                next_attempt,
                crate::db::truncate_str(err, 2000),
                now,
                task_id,
                lease_owner
            ],
        )?;
        return ensure_task_updated(updated, task_id);
    }

    let updated = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'pending',
             attempts = ?1,
             next_retry_epoch = ?2,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             last_error = ?3,
             updated_at_epoch = ?4
         WHERE id = ?5 AND lease_owner = ?6 AND status = 'processing'",
        params![
            next_attempt,
            now + backoff_secs.max(1),
            crate::db::truncate_str(err, 2000),
            now,
            task_id,
            lease_owner
        ],
    )?;
    ensure_task_updated(updated, task_id)
}

fn load_claimed_extraction_task(conn: &Connection, task_id: i64) -> Result<ExtractionTask> {
    let row = conn.query_row(
        "SELECT t.id, t.task_kind, t.host_id, t.workspace_id, t.project_id, t.session_row_id,
                h.name, p.project_path, s.session_id,
                t.priority, t.cursor_event_id, t.high_watermark_event_id, t.attempts
         FROM extraction_tasks t
         JOIN hosts h ON h.id = t.host_id
         JOIN projects p ON p.id = t.project_id
         LEFT JOIN sessions s ON s.id = t.session_row_id
         WHERE t.id = ?1",
        params![task_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, i64>(12)?,
            ))
        },
    )?;

    let ai_profile = load_task_ai_profile(conn, row.2, row.4, row.5, row.11)?;
    Ok(ExtractionTask {
        id: row.0,
        task_kind: ExtractionTaskKind::from_db(&row.1)?,
        host_id: row.2,
        workspace_id: row.3,
        project_id: row.4,
        session_row_id: row.5,
        host: row.6,
        project: row.7,
        session_id: row.8,
        ai_profile,
        priority: row.9,
        cursor_event_id: row.10,
        high_watermark_event_id: row.11,
        attempts: row.12,
    })
}

fn load_task_ai_profile(
    conn: &Connection,
    host_id: i64,
    project_id: i64,
    session_row_id: Option<i64>,
    high_watermark_event_id: Option<i64>,
) -> Result<Option<String>> {
    let Some(session_row_id) = session_row_id else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT COALESCE(
                    CASE
                        WHEN b.content_encoding = 'plain' THEN CAST(b.content_bytes AS TEXT)
                        ELSE NULL
                    END,
                    e.content_text,
                    ''
                ) AS content
         FROM captured_events e
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         WHERE e.host_id = ?1
           AND e.project_id = ?2
           AND e.session_row_id = ?3
           AND (?4 IS NULL OR e.id <= ?4)
         ORDER BY e.id DESC",
    )?;
    let contents = stmt
        .query_map(
            params![host_id, project_id, session_row_id, high_watermark_event_id],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(contents
        .iter()
        .find_map(|content| crate::runtime_config::profile_from_payload_text(content)))
}

fn ensure_task_updated(updated: usize, task_id: i64) -> Result<()> {
    if updated == 0 {
        bail!("extraction task {task_id} is not leased by this worker");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use crate::db::{record_captured_event, CaptureEventInput};

    use super::*;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        crate::migrate::run_migrations(&conn).expect("migrations should run");
        conn
    }

    fn insert_task(
        conn: &Connection,
        session_id: &str,
        task_kind: ExtractionTaskKind,
    ) -> Result<i64> {
        let outcome = record_captured_event(
            conn,
            &CaptureEventInput {
                host: "codex-cli",
                session_id,
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Task"),
                content: session_id,
                task_kind: Some(task_kind),
            },
        )?;
        outcome
            .extraction_task_id
            .ok_or_else(|| anyhow::anyhow!("expected extraction task id"))
    }

    fn task_status(conn: &Connection, task_id: i64) -> (String, i64, Option<i64>, Option<String>) {
        conn.query_row(
            "SELECT status, attempts, next_retry_epoch, last_error
             FROM extraction_tasks WHERE id = ?1",
            params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("task state should query")
    }

    #[test]
    fn claim_next_extraction_task_orders_by_priority_and_age() {
        let mut conn = setup_conn();
        let observation_id = insert_task(
            &conn,
            "sess-observation",
            ExtractionTaskKind::ObservationExtract,
        )
        .expect("observation task should insert");
        let rollup_id = insert_task(&conn, "sess-rollup", ExtractionTaskKind::SessionRollup)
            .expect("rollup task should insert");

        let claimed = claim_next_extraction_task(&mut conn, "worker-a", 60)
            .expect("claim should succeed")
            .expect("task should be claimed");

        assert_eq!(claimed.id, rollup_id);
        assert_eq!(claimed.task_kind, ExtractionTaskKind::SessionRollup);
        assert_eq!(claimed.host, "codex-cli");
        assert_eq!(claimed.session_id.as_deref(), Some("sess-rollup"));

        let status = task_status(&conn, observation_id).0;
        assert_eq!(status, "pending");
    }

    #[test]
    fn claim_next_extraction_task_preserves_ai_profile_from_capture_payload() -> Result<()> {
        let mut conn = setup_conn();
        record_captured_event(
            &conn,
            &CaptureEventInput {
                host: "codex-cli",
                session_id: "sess-profile",
                project: "/tmp/remem",
                cwd: None,
                event_type: "session_stop",
                role: None,
                tool_name: None,
                content: r#"{"session_id":"sess-profile","remem_ai_profile":"custom"}"#,
                task_kind: Some(ExtractionTaskKind::SessionRollup),
            },
        )?;

        let claimed = claim_next_extraction_task(&mut conn, "worker-a", 60)?
            .ok_or_else(|| anyhow::anyhow!("task should exist"))?;

        assert_eq!(claimed.ai_profile.as_deref(), Some("custom"));
        Ok(())
    }

    #[test]
    fn claim_next_extraction_task_reads_ai_profile_from_large_capture_blob() -> Result<()> {
        let mut conn = setup_conn();
        let content = format!(
            r#"{{"session_id":"sess-large-profile","prefix":"{}","remem_ai_profile":"large-custom","suffix":"{}"}}"#,
            "a".repeat(10 * 1024),
            "b".repeat(10 * 1024)
        );
        record_captured_event(
            &conn,
            &CaptureEventInput {
                host: "codex-cli",
                session_id: "sess-large-profile",
                project: "/tmp/remem",
                cwd: None,
                event_type: "session_stop",
                role: None,
                tool_name: None,
                content: &content,
                task_kind: Some(ExtractionTaskKind::SessionRollup),
            },
        )?;

        let claimed = claim_next_extraction_task(&mut conn, "worker-a", 60)?
            .ok_or_else(|| anyhow::anyhow!("task should exist"))?;

        assert_eq!(claimed.ai_profile.as_deref(), Some("large-custom"));
        Ok(())
    }

    #[test]
    fn claim_next_extraction_task_does_not_double_claim_active_task() {
        let mut conn = setup_conn();
        let task_id = insert_task(&conn, "sess-single", ExtractionTaskKind::SessionRollup)
            .expect("task should insert");

        let first = claim_next_extraction_task(&mut conn, "worker-a", 60)
            .expect("first claim should succeed")
            .expect("first claim should return task");
        let second = claim_next_extraction_task(&mut conn, "worker-b", 60)
            .expect("second claim should succeed");

        assert_eq!(first.id, task_id);
        assert!(second.is_none());
    }

    #[test]
    fn release_expired_extraction_task_leases_requeues_only_expired_tasks() {
        let mut conn = setup_conn();
        let expired_id = insert_task(&conn, "sess-expired", ExtractionTaskKind::SessionRollup)
            .expect("expired task should insert");
        let fresh_id = insert_task(&conn, "sess-fresh", ExtractionTaskKind::ObservationExtract)
            .expect("fresh task should insert");

        claim_next_extraction_task(&mut conn, "worker-expired", 60)
            .expect("expired worker claim should succeed")
            .expect("expired task should be claimed");
        claim_next_extraction_task(&mut conn, "worker-fresh", 60)
            .expect("fresh worker claim should succeed")
            .expect("fresh task should be claimed");
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE extraction_tasks
             SET lease_expires_epoch = ?1
             WHERE id = ?2",
            params![now - 1, expired_id],
        )
        .expect("expired lease should update");

        let released =
            release_expired_extraction_task_leases(&conn).expect("release should succeed");

        assert_eq!(released, 1);
        assert_eq!(task_status(&conn, expired_id).0, "pending");
        assert_eq!(task_status(&conn, fresh_id).0, "processing");
    }

    #[test]
    fn mark_extraction_task_done_clears_lease_and_advances_cursor() {
        let mut conn = setup_conn();
        let task_id = insert_task(&conn, "sess-done", ExtractionTaskKind::SessionRollup)
            .expect("task should insert");
        let task = claim_next_extraction_task(&mut conn, "worker-a", 60)
            .expect("claim should succeed")
            .expect("task should be claimed");

        mark_extraction_task_done(&conn, task.id, "worker-a", task.high_watermark_event_id)
            .expect("done should succeed");

        let (status, lease_owner, cursor, high_watermark): (
            String,
            Option<String>,
            Option<i64>,
            Option<i64>,
        ) = conn
            .query_row(
                "SELECT status, lease_owner, cursor_event_id, high_watermark_event_id
                 FROM extraction_tasks WHERE id = ?1",
                params![task_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("task should query");
        assert_eq!(status, "done");
        assert!(lease_owner.is_none());
        assert_eq!(cursor, high_watermark);
    }

    #[test]
    fn mark_extraction_task_done_requeues_when_watermark_advanced_after_claim() {
        let mut conn = setup_conn();
        let task_id = insert_task(&conn, "sess-coalesce", ExtractionTaskKind::SessionRollup)
            .expect("task should insert");
        let task = claim_next_extraction_task(&mut conn, "worker-a", 60)
            .expect("claim should succeed")
            .expect("task should be claimed");
        let claimed_high_watermark = task.high_watermark_event_id;
        insert_task(&conn, "sess-coalesce", ExtractionTaskKind::SessionRollup)
            .expect("coalesced task should update high watermark");

        mark_extraction_task_done(&conn, task.id, "worker-a", claimed_high_watermark)
            .expect("done should succeed");

        let (status, lease_owner, cursor, high_watermark): (
            String,
            Option<String>,
            Option<i64>,
            Option<i64>,
        ) = conn
            .query_row(
                "SELECT status, lease_owner, cursor_event_id, high_watermark_event_id
                 FROM extraction_tasks WHERE id = ?1",
                params![task_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("task should query");
        assert_eq!(status, "pending");
        assert!(lease_owner.is_none());
        assert_eq!(cursor, claimed_high_watermark);
        assert!(high_watermark > cursor);
    }

    #[test]
    fn mark_extraction_task_failed_or_retry_keeps_retryable_task_visible() {
        let mut conn = setup_conn();
        let task_id = insert_task(&conn, "sess-retry", ExtractionTaskKind::SessionRollup)
            .expect("task should insert");
        claim_next_extraction_task(&mut conn, "worker-a", 60)
            .expect("claim should succeed")
            .expect("task should be claimed");

        mark_extraction_task_failed_or_retry(&conn, task_id, "worker-a", "temporary", 30)
            .expect("retry should succeed");

        let (status, attempts, next_retry, last_error) = task_status(&conn, task_id);
        assert_eq!(status, "pending");
        assert_eq!(attempts, 1);
        assert!(next_retry.is_some());
        assert_eq!(last_error.as_deref(), Some("temporary"));
    }

    #[test]
    fn mark_extraction_task_failed_or_retry_exhausts_after_max_attempts() {
        let mut conn = setup_conn();
        let task_id = insert_task(&conn, "sess-failed", ExtractionTaskKind::SessionRollup)
            .expect("task should insert");
        conn.execute(
            "UPDATE extraction_tasks SET attempts = ?1 WHERE id = ?2",
            params![EXTRACTION_TASK_MAX_ATTEMPTS - 1, task_id],
        )
        .expect("attempt count should update");
        claim_next_extraction_task(&mut conn, "worker-a", 60)
            .expect("claim should succeed")
            .expect("task should be claimed");

        mark_extraction_task_failed_or_retry(&conn, task_id, "worker-a", "exhausted", 30)
            .expect("failure should succeed");

        let (status, attempts, next_retry, last_error) = task_status(&conn, task_id);
        assert_eq!(status, "failed");
        assert_eq!(attempts, EXTRACTION_TASK_MAX_ATTEMPTS);
        assert!(next_retry.is_none());
        assert_eq!(last_error.as_deref(), Some("exhausted"));
    }

    #[test]
    fn mark_extraction_task_failed_records_permanent_failure_without_retry() {
        let mut conn = setup_conn();
        let task_id = insert_task(&conn, "sess-permanent", ExtractionTaskKind::SessionRollup)
            .expect("task should insert");
        claim_next_extraction_task(&mut conn, "worker-a", 60)
            .expect("claim should succeed")
            .expect("task should be claimed");

        mark_extraction_task_failed(&conn, task_id, "worker-a", "not implemented")
            .expect("permanent failure should succeed");

        let (status, attempts, next_retry, last_error) = task_status(&conn, task_id);
        assert_eq!(status, "failed");
        assert_eq!(attempts, 1);
        assert!(next_retry.is_none());
        assert_eq!(last_error.as_deref(), Some("not implemented"));
    }

    #[test]
    fn defer_extraction_task_requeues_and_increments_attempts() {
        let mut conn = setup_conn();
        let task_id = insert_task(&conn, "sess-defer", ExtractionTaskKind::SessionRollup)
            .expect("task should insert");
        claim_next_extraction_task(&mut conn, "worker-a", 60)
            .expect("claim should succeed")
            .expect("task should be claimed");

        defer_extraction_task(&conn, task_id, "worker-a", "not implemented", 30)
            .expect("defer should succeed");

        let (status, attempts, next_retry, last_error) = task_status(&conn, task_id);
        assert_eq!(status, "pending");
        assert_eq!(attempts, 1);
        assert!(next_retry.is_some());
        assert_eq!(last_error.as_deref(), Some("not implemented"));
    }

    #[test]
    fn defer_extraction_task_exhausts_after_max_attempts() {
        let mut conn = setup_conn();
        let task_id = insert_task(
            &conn,
            "sess-defer-exhaust",
            ExtractionTaskKind::SessionRollup,
        )
        .expect("task should insert");
        conn.execute(
            "UPDATE extraction_tasks SET attempts = ?1 WHERE id = ?2",
            params![EXTRACTION_TASK_MAX_ATTEMPTS - 1, task_id],
        )
        .expect("attempt count should update");
        claim_next_extraction_task(&mut conn, "worker-a", 60)
            .expect("claim should succeed")
            .expect("task should be claimed");

        defer_extraction_task(&conn, task_id, "worker-a", "still ambiguous", 30)
            .expect("defer should exhaust");

        let (status, attempts, next_retry, last_error) = task_status(&conn, task_id);
        assert_eq!(status, "failed");
        assert_eq!(attempts, EXTRACTION_TASK_MAX_ATTEMPTS);
        assert!(next_retry.is_none());
        assert_eq!(last_error.as_deref(), Some("still ambiguous"));
    }
}
