use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

/// Count tasks ready to be claimed under `(host_id, project_id)`. "Ready"
/// matches the same predicate as `claim_ready_tasks`: status pending /
/// delayed and `next_retry_epoch` past.
pub fn count_ready_for_identity(
    conn: &Connection,
    host_id: i64,
    project_id: i64,
    now: i64,
) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM extraction_tasks
         WHERE host_id = ?1
           AND project_id = ?2
           AND status IN ('pending', 'delayed')
           AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?3)",
        params![host_id, project_id, now],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Earliest `created_at_epoch` among ready tasks for this identity, or
/// `None` when nothing is ready. Useful for status / doctor age signals.
pub fn oldest_ready_epoch(
    conn: &Connection,
    host_id: i64,
    project_id: i64,
    now: i64,
) -> Result<Option<i64>> {
    let row = conn
        .query_row(
            "SELECT MIN(created_at_epoch) FROM extraction_tasks
             WHERE host_id = ?1
               AND project_id = ?2
               AND status IN ('pending', 'delayed')
               AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?3)",
            params![host_id, project_id, now],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?;
    Ok(row.flatten())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::open_at as open_schema_at;
    use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path};
    use crate::extraction::claim::mark_task_delayed;
    use crate::extraction::enqueue::{enqueue_extraction_task, EnqueueRequest};
    use crate::extraction::types::TaskKind;

    fn fresh() -> (Connection, std::path::PathBuf, i64, i64, i64) {
        let path = unique_temp_db_path("extr-q");
        let conn = open_schema_at(&path).unwrap();
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
        let proj_id = conn.last_insert_rowid();
        let host_id: i64 = conn
            .query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |r| {
                r.get(0)
            })
            .unwrap();
        (conn, path, host_id, ws_id, proj_id)
    }

    fn enq(conn: &Connection, h: i64, w: i64, p: i64, key: &str, now: i64) -> i64 {
        enqueue_extraction_task(
            conn,
            EnqueueRequest {
                task_kind: TaskKind::SessionRollup,
                host_id: h,
                workspace_id: w,
                project_id: p,
                session_row_id: None,
                priority: 100,
                idempotency_key: key,
                high_watermark_event_id: Some(1),
                now,
            },
        )
        .unwrap()
    }

    #[test]
    fn count_ready_returns_zero_for_empty_table() {
        let (conn, path, h, _, p) = fresh();
        assert_eq!(count_ready_for_identity(&conn, h, p, 1_000).unwrap(), 0);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn count_ready_includes_pending_and_due_delayed() {
        let (conn, path, h, w, p) = fresh();
        enq(&conn, h, w, p, "pending-1", 100);
        enq(&conn, h, w, p, "pending-2", 200);
        let id = enq(&conn, h, w, p, "delayed", 300);
        mark_task_delayed(&conn, id, 5_000, "transient", 400).unwrap();

        // Before retry epoch: only the 2 pending rows count.
        assert_eq!(count_ready_for_identity(&conn, h, p, 4_000).unwrap(), 2);
        // After retry epoch: delayed becomes ready.
        assert_eq!(count_ready_for_identity(&conn, h, p, 6_000).unwrap(), 3);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn oldest_ready_returns_earliest_created_at() {
        let (conn, path, h, w, p) = fresh();
        enq(&conn, h, w, p, "newer", 500);
        enq(&conn, h, w, p, "oldest", 100);
        enq(&conn, h, w, p, "middle", 300);
        let oldest = oldest_ready_epoch(&conn, h, p, 1_000).unwrap();
        assert_eq!(oldest, Some(100));
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn oldest_ready_returns_none_when_empty() {
        let (conn, path, h, _, p) = fresh();
        assert_eq!(oldest_ready_epoch(&conn, h, p, 1_000).unwrap(), None);
        cleanup_temp_db_files(&path);
    }
}
