use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::IdentityIds;
use crate::db::ExtractionTaskKind;

pub(super) fn with_capture_savepoint<T>(
    conn: &Connection,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    conn.execute_batch("SAVEPOINT remem_capture_event_task")
        .context("start capture event/task savepoint")?;
    match operation() {
        Ok(value) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_capture_event_task")
                .context("release capture event/task savepoint")?;
            Ok(value)
        }
        Err(error) => match conn.execute_batch(
            "ROLLBACK TO SAVEPOINT remem_capture_event_task;
             RELEASE SAVEPOINT remem_capture_event_task;",
        ) {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(error.context(format!(
                "capture event/task savepoint rollback also failed: {rollback_error}"
            ))),
        },
    }
}

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

pub(super) fn extraction_task_for_replayed_event(
    conn: &Connection,
    identity: IdentityIds,
    kind: ExtractionTaskKind,
    event_row_id: i64,
    late_git_evidence_key: Option<&str>,
    now: i64,
) -> Result<i64> {
    let existing = conn
        .query_row(
            "SELECT id, status, cursor_event_id
         FROM extraction_tasks
         WHERE idempotency_key = ?1",
            params![extraction_task_idempotency_key(identity, kind)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((task_id, status, cursor_event_id)) = existing else {
        return coalesce_extraction_task(conn, identity, kind, event_row_id, now);
    };
    let Some(evidence_key) = late_git_evidence_key else {
        return Ok(task_id);
    };

    if status != "processing" && cursor_event_id.unwrap_or(0) < event_row_id {
        return coalesce_extraction_task(conn, identity, kind, event_row_id, now);
    }

    enqueue_late_git_evidence_task(conn, identity, event_row_id, evidence_key, now)
}

fn enqueue_late_git_evidence_task(
    conn: &Connection,
    identity: IdentityIds,
    event_row_id: i64,
    evidence_key: &str,
    now: i64,
) -> Result<i64> {
    let kind = ExtractionTaskKind::CapturedGitLink;
    let idempotency_key = format!(
        "{}:{}:{}:{}:late-git-evidence:{}:{}",
        identity.host_id,
        identity.project_id,
        identity.session_row_id,
        kind.as_str(),
        event_row_id,
        evidence_key
    );
    conn.execute(
        "INSERT INTO extraction_tasks
         (task_kind, host_id, workspace_id, project_id, session_row_id, priority, status,
          idempotency_key, cursor_event_id, high_watermark_event_id, attempts,
          next_retry_epoch, lease_owner, lease_expires_epoch, last_error, created_at_epoch,
          updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?9, 0, NULL, NULL, NULL, NULL,
                 ?10, ?10)
         ON CONFLICT(idempotency_key) DO NOTHING",
        params![
            kind.as_str(),
            identity.host_id,
            identity.workspace_id,
            identity.project_id,
            identity.session_row_id,
            kind.priority(),
            idempotency_key,
            event_row_id.saturating_sub(1),
            event_row_id,
            now
        ],
    )?;
    conn.query_row(
        "SELECT id FROM extraction_tasks WHERE idempotency_key = ?1",
        params![idempotency_key],
        |row| row.get(0),
    )
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

    fn capture_counts(conn: &Connection) -> Result<(i64, i64, i64, i64, i64, i64, i64)> {
        Ok((
            conn.query_row("SELECT COUNT(*) FROM hosts", [], |row| row.get(0))?,
            conn.query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0))?,
            conn.query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))?,
            conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?,
            conn.query_row("SELECT COUNT(*) FROM event_blobs", [], |row| row.get(0))?,
            conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?,
            conn.query_row("SELECT COUNT(*) FROM extraction_tasks", [], |row| {
                row.get(0)
            })?,
        ))
    }

    #[test]
    fn task_insert_failure_rolls_back_capture_event_and_identity() -> Result<()> {
        let conn = setup_conn();
        let baseline = capture_counts(&conn)?;
        conn.execute_batch(
            "CREATE TRIGGER fail_capture_task
             BEFORE INSERT ON extraction_tasks
             BEGIN
               SELECT RAISE(FAIL, 'forced capture task failure');
             END;",
        )?;
        let large_content = "evidence".repeat(4_000);
        let input = CaptureEventInput {
            host: "codex-cli",
            session_id: "savepoint-session",
            project: "/tmp/remem-savepoint",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: &large_content,
            task_kind: Some(ExtractionTaskKind::ObservationExtract),
        };

        let error = record_captured_event_with_id(&conn, &input, Some("savepoint-event"))
            .expect_err("task failure must roll back capture unit");
        assert!(error.to_string().contains("forced capture task failure"));
        assert_eq!(capture_counts(&conn)?, baseline);

        conn.execute_batch("DROP TRIGGER fail_capture_task")?;
        let outcome = record_captured_event_with_id(&conn, &input, Some("savepoint-event"))?;
        assert!(outcome.extraction_task_id.is_some());
        assert_eq!(capture_counts(&conn)?.5, baseline.5 + 1);
        assert_eq!(capture_counts(&conn)?.6, baseline.6 + 1);
        Ok(())
    }

    #[test]
    fn capture_savepoint_nests_inside_outer_transaction() -> Result<()> {
        let conn = setup_conn();
        let baseline = capture_counts(&conn)?;
        let tx = conn.unchecked_transaction()?;
        record_captured_event_with_id(
            &tx,
            &CaptureEventInput {
                host: "codex-cli",
                session_id: "nested-savepoint-session",
                project: "/tmp/remem-savepoint",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: "nested capture",
                task_kind: Some(ExtractionTaskKind::ObservationExtract),
            },
            Some("nested-savepoint-event"),
        )?;
        tx.rollback()?;
        assert_eq!(capture_counts(&conn)?, baseline);
        Ok(())
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
