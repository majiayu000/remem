use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::types::TaskKind;

/// Inputs for `enqueue_extraction_task`. `idempotency_key` carries the
/// uniqueness contract — repeated enqueues with the same key coalesce
/// onto the existing row instead of creating duplicates.
pub struct EnqueueRequest<'a> {
    pub task_kind: TaskKind,
    pub host_id: i64,
    pub workspace_id: i64,
    pub project_id: i64,
    pub session_row_id: Option<i64>,
    pub priority: i64,
    pub idempotency_key: &'a str,
    pub high_watermark_event_id: Option<i64>,
    pub now: i64,
}

/// Insert or coalesce an extraction task. Idempotent on `idempotency_key`:
/// when a row already exists, only `high_watermark_event_id` is bumped (and
/// only when the new value is strictly greater than the current). Returns
/// the row id of the new or existing task.
pub fn enqueue_extraction_task(conn: &Connection, req: EnqueueRequest) -> Result<i64> {
    if let Some((id, current_hwm)) = conn
        .query_row(
            "SELECT id, high_watermark_event_id FROM extraction_tasks
             WHERE idempotency_key = ?1",
            [req.idempotency_key],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
        )
        .optional()?
    {
        let bumped = match (current_hwm, req.high_watermark_event_id) {
            (Some(curr), Some(new)) if new > curr => Some(new),
            (None, Some(new)) => Some(new),
            _ => None,
        };
        if let Some(new_hwm) = bumped {
            conn.execute(
                "UPDATE extraction_tasks
                 SET high_watermark_event_id = ?1, updated_at_epoch = ?2
                 WHERE id = ?3",
                params![new_hwm, req.now, id],
            )?;
        }
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO extraction_tasks(
            task_kind, host_id, workspace_id, project_id, session_row_id,
            priority, status, idempotency_key,
            cursor_event_id, high_watermark_event_id,
            attempts, next_retry_epoch, lease_owner, lease_expires_epoch,
            last_error, created_at_epoch, updated_at_epoch
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5,
            ?6, 'pending', ?7,
            NULL, ?8,
            0, NULL, NULL, NULL,
            NULL, ?9, ?9
         )",
        params![
            req.task_kind.as_db_value(),
            req.host_id,
            req.workspace_id,
            req.project_id,
            req.session_row_id,
            req.priority,
            req.idempotency_key,
            req.high_watermark_event_id,
            req.now,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::open_at as open_schema_at;
    use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path};

    fn fresh() -> (Connection, std::path::PathBuf) {
        let path = unique_temp_db_path("extr-enq");
        let conn = open_schema_at(&path).unwrap();
        // Seed minimal FK rows.
        conn.execute(
            "INSERT INTO workspaces(root_path, created_at_epoch, updated_at_epoch)
             VALUES ('/tmp/repo', 0, 0)",
            [],
        )
        .unwrap();
        let ws_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO projects(workspace_id, project_path, project_key,
                created_at_epoch, updated_at_epoch)
             VALUES (?1, '/tmp/repo', '/tmp/repo', 0, 0)",
            [ws_id],
        )
        .unwrap();
        (conn, path)
    }

    fn host_codex(conn: &Connection) -> i64 {
        conn.query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |r| {
            r.get(0)
        })
        .unwrap()
    }

    fn ids(conn: &Connection) -> (i64, i64, i64) {
        let host_id = host_codex(conn);
        let ws_id: i64 = conn
            .query_row("SELECT id FROM workspaces LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let proj_id: i64 = conn
            .query_row("SELECT id FROM projects LIMIT 1", [], |r| r.get(0))
            .unwrap();
        (host_id, ws_id, proj_id)
    }

    #[test]
    fn enqueue_inserts_pending_row() {
        let (conn, path) = fresh();
        let (h, w, p) = ids(&conn);
        let id = enqueue_extraction_task(
            &conn,
            EnqueueRequest {
                task_kind: TaskKind::SessionRollup,
                host_id: h,
                workspace_id: w,
                project_id: p,
                session_row_id: None,
                priority: 100,
                idempotency_key: "k1",
                high_watermark_event_id: Some(42),
                now: 1_000,
            },
        )
        .unwrap();
        let (status, hwm, attempts): (String, i64, i64) = conn
            .query_row(
                "SELECT status, high_watermark_event_id, attempts FROM extraction_tasks WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(hwm, 42);
        assert_eq!(attempts, 0);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn enqueue_dedupes_on_idempotency_key() {
        let (conn, path) = fresh();
        let (h, w, p) = ids(&conn);
        let req = |hwm| EnqueueRequest {
            task_kind: TaskKind::SessionRollup,
            host_id: h,
            workspace_id: w,
            project_id: p,
            session_row_id: None,
            priority: 100,
            idempotency_key: "shared",
            high_watermark_event_id: Some(hwm),
            now: 1_000,
        };
        let id1 = enqueue_extraction_task(&conn, req(10)).unwrap();
        let id2 = enqueue_extraction_task(&conn, req(20)).unwrap();
        assert_eq!(id1, id2);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM extraction_tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let hwm: i64 = conn
            .query_row(
                "SELECT high_watermark_event_id FROM extraction_tasks WHERE id = ?1",
                [id1],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hwm, 20, "second enqueue with larger hwm bumps the row");
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn enqueue_does_not_lower_existing_hwm() {
        let (conn, path) = fresh();
        let (h, w, p) = ids(&conn);
        let req = |hwm| EnqueueRequest {
            task_kind: TaskKind::SessionRollup,
            host_id: h,
            workspace_id: w,
            project_id: p,
            session_row_id: None,
            priority: 100,
            idempotency_key: "lower",
            high_watermark_event_id: Some(hwm),
            now: 1_000,
        };
        enqueue_extraction_task(&conn, req(50)).unwrap();
        enqueue_extraction_task(&conn, req(30)).unwrap();
        let hwm: i64 = conn
            .query_row(
                "SELECT high_watermark_event_id FROM extraction_tasks",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hwm, 50, "smaller hwm must not regress");
        cleanup_temp_db_files(&path);
    }
}
