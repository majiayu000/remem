use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::IdentityIds;
use crate::db::ExtractionTaskKind;

pub(super) fn coalesce_extraction_task(
    conn: &Connection,
    identity: IdentityIds,
    kind: ExtractionTaskKind,
    event_row_id: i64,
    now: i64,
) -> Result<i64> {
    let idempotency_key = extraction_task_idempotency_key(identity, kind);
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
             -- Reviving a terminal task resets its retry budget: the old
             -- attempts counted a range the exhaust path already skipped, so
             -- the new range must start with fresh attempts or it would fail
             -- terminally on its first defer.
             attempts = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN 0
                 ELSE extraction_tasks.attempts
             END,
             next_retry_epoch = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.next_retry_epoch
             END,
             last_error = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.last_error
             END,
             failure_class = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.failure_class
             END,
             failed_at_epoch = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.failed_at_epoch
             END,
             archived_at_epoch = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.archived_at_epoch
             END,
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            kind.as_str(),
            identity.host_id,
            identity.workspace_id,
            identity.project_id,
            identity.session_row_id,
            kind.priority(),
            idempotency_key,
            event_row_id,
            now
        ],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM extraction_tasks WHERE idempotency_key = ?1",
        params![idempotency_key],
        |row| row.get(0),
    )?)
}

pub(super) fn existing_extraction_task_id(
    conn: &Connection,
    identity: IdentityIds,
    kind: ExtractionTaskKind,
) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM extraction_tasks WHERE idempotency_key = ?1",
        params![extraction_task_idempotency_key(identity, kind)],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn extraction_task_idempotency_key(identity: IdentityIds, kind: ExtractionTaskKind) -> String {
    format!(
        "{}:{}:{}:{}",
        identity.host_id,
        identity.project_id,
        identity.session_row_id,
        kind.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::capture::{record_captured_event_with_id, CaptureEventInput};

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        crate::migrate::run_migrations(&conn).expect("migrations should run");
        conn
    }

    #[test]
    fn duplicate_fixed_event_id_does_not_revive_done_task() -> Result<()> {
        let conn = setup_conn();
        let input = CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-fixed-replay",
            project: "/tmp/remem",
            cwd: Some("/tmp/remem"),
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content: r#"{"session_id":"sess-fixed-replay"}"#,
            task_kind: Some(ExtractionTaskKind::SessionRollup),
        };
        let first =
            record_captured_event_with_id(&conn, &input, Some("session_stop-spill-stable"))?;
        let task_id = first
            .extraction_task_id
            .expect("first fixed capture should coalesce a task");
        conn.execute(
            "UPDATE extraction_tasks
             SET status = 'done',
                 cursor_event_id = high_watermark_event_id,
                 lease_owner = NULL,
                 lease_expires_epoch = NULL
             WHERE id = ?1",
            params![task_id],
        )?;

        let second =
            record_captured_event_with_id(&conn, &input, Some("session_stop-spill-stable"))?;

        assert_eq!(second.event_row_id, first.event_row_id);
        assert_eq!(second.extraction_task_id, Some(task_id));
        let (status, cursor, high_watermark, event_count): (String, i64, i64, i64) = conn
            .query_row(
                "SELECT t.status,
                        t.cursor_event_id,
                        t.high_watermark_event_id,
                        (SELECT COUNT(*) FROM captured_events)
                 FROM extraction_tasks t
                 WHERE t.id = ?1",
                params![task_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        assert_eq!(status, "done");
        assert_eq!(cursor, first.event_row_id);
        assert_eq!(high_watermark, first.event_row_id);
        assert_eq!(event_count, 1);
        Ok(())
    }
}
