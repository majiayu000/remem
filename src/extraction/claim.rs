use anyhow::Result;
use rusqlite::{params, Connection};

use super::types::TaskKind;

/// Default lease duration. Short enough that crashed workers do not block
/// progress for long; long enough that healthy workers do not get racy
/// re-claims under normal AI batch latency.
pub const DEFAULT_LEASE_SECS: i64 = 600;

/// Minimal projection returned to the worker after a successful claim.
pub struct ClaimedTask {
    pub id: i64,
    pub task_kind: TaskKind,
    pub host_id: i64,
    pub project_id: i64,
    pub session_row_id: Option<i64>,
    pub cursor_event_id: Option<i64>,
    pub high_watermark_event_id: Option<i64>,
    pub attempts: i64,
}

/// Claim up to `limit` ready tasks under `(host_id, project_id)`. Ready
/// means status is `pending` or `delayed` and `next_retry_epoch` has
/// passed; expired processing rows must be recovered first via
/// `recover_expired_leases`. Sets the lease, bumps attempts, transitions
/// to `processing`, and returns the projected rows.
pub fn claim_ready_tasks(
    conn: &Connection,
    host_id: i64,
    project_id: i64,
    lease_owner: &str,
    lease_secs: i64,
    limit: usize,
    now: i64,
) -> Result<Vec<ClaimedTask>> {
    let lease_expires = now + lease_secs;
    let candidate_ids: Vec<i64> = conn
        .prepare(
            "SELECT id FROM extraction_tasks
             WHERE host_id = ?1
               AND project_id = ?2
               AND status IN ('pending', 'delayed')
               AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?3)
               AND (lease_owner IS NULL OR lease_expires_epoch IS NULL
                    OR lease_expires_epoch < ?3)
             ORDER BY priority ASC, created_at_epoch ASC, id ASC
             LIMIT ?4",
        )?
        .query_map(
            params![host_id, project_id, now, limit as i64],
            |row| row.get::<_, i64>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;

    let mut claimed = Vec::with_capacity(candidate_ids.len());
    for id in candidate_ids {
        let updated = conn.execute(
            "UPDATE extraction_tasks
             SET status = 'processing',
                 lease_owner = ?1,
                 lease_expires_epoch = ?2,
                 attempts = attempts + 1,
                 next_retry_epoch = NULL,
                 updated_at_epoch = ?3
             WHERE id = ?4
               AND status IN ('pending', 'delayed')",
            params![lease_owner, lease_expires, now, id],
        )?;
        if updated == 0 {
            // Lost race — another worker claimed in between SELECT and UPDATE.
            continue;
        }
        let task = conn.query_row(
            "SELECT id, task_kind, host_id, project_id, session_row_id,
                    cursor_event_id, high_watermark_event_id, attempts
             FROM extraction_tasks WHERE id = ?1",
            [id],
            |row| {
                let kind_str: String = row.get(1)?;
                let task_kind = TaskKind::parse(&kind_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())),
                    )
                })?;
                Ok(ClaimedTask {
                    id: row.get(0)?,
                    task_kind,
                    host_id: row.get(2)?,
                    project_id: row.get(3)?,
                    session_row_id: row.get(4)?,
                    cursor_event_id: row.get(5)?,
                    high_watermark_event_id: row.get(6)?,
                    attempts: row.get(7)?,
                })
            },
        )?;
        claimed.push(task);
    }
    Ok(claimed)
}

/// Reset processing rows whose lease has expired back to `pending` so
/// future `claim_ready_tasks` calls can pick them up. Returns the number
/// of rows recovered.
pub fn recover_expired_leases(conn: &Connection, now: i64) -> Result<usize> {
    let n = conn.execute(
        "UPDATE extraction_tasks
         SET status = 'pending',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             updated_at_epoch = ?1
         WHERE status = 'processing'
           AND lease_expires_epoch IS NOT NULL
           AND lease_expires_epoch < ?1",
        [now],
    )?;
    Ok(n)
}

/// Mark a task done. `new_cursor` is the event id range advanced by this
/// run (per v2.1 §1 M4 progress invariant: progress is event-range-based,
/// not observation-count-based). When `None`, leaves the cursor alone.
pub fn mark_task_done(
    conn: &Connection,
    id: i64,
    new_cursor: Option<i64>,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'done',
             cursor_event_id = COALESCE(?1, cursor_event_id),
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             last_error = NULL,
             updated_at_epoch = ?2
         WHERE id = ?3",
        params![new_cursor, now, id],
    )?;
    Ok(())
}

/// Mark a task delayed (transient failure). Sets `next_retry_epoch`,
/// records `last_error`, and clears the lease.
pub fn mark_task_delayed(
    conn: &Connection,
    id: i64,
    next_retry_epoch: i64,
    last_error: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'delayed',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             next_retry_epoch = ?1,
             last_error = ?2,
             updated_at_epoch = ?3
         WHERE id = ?4",
        params![next_retry_epoch, last_error, now, id],
    )?;
    Ok(())
}

/// Mark a task permanently failed. Lease is cleared.
pub fn mark_task_failed(conn: &Connection, id: i64, last_error: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'failed',
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             last_error = ?1,
             updated_at_epoch = ?2
         WHERE id = ?3",
        params![last_error, now, id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path};
    use crate::extraction::enqueue::{enqueue_extraction_task, EnqueueRequest};
    use crate::v2_db::open_v2_db_at;

    fn fresh() -> (Connection, std::path::PathBuf, i64, i64, i64) {
        let path = unique_temp_db_path("extr-claim");
        let conn = open_v2_db_at(&path).unwrap();
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
            .query_row(
                "SELECT id FROM hosts WHERE name = 'codex-cli'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        (conn, path, host_id, ws_id, proj_id)
    }

    fn enq(conn: &Connection, h: i64, w: i64, p: i64, key: &str, prio: i64, now: i64) -> i64 {
        enqueue_extraction_task(
            conn,
            EnqueueRequest {
                task_kind: TaskKind::SessionRollup,
                host_id: h,
                workspace_id: w,
                project_id: p,
                session_row_id: None,
                priority: prio,
                idempotency_key: key,
                high_watermark_event_id: Some(1),
                now,
            },
        )
        .unwrap()
    }

    #[test]
    fn claim_returns_ready_in_priority_order() {
        let (conn, path, h, w, p) = fresh();
        enq(&conn, h, w, p, "low", 200, 100);
        enq(&conn, h, w, p, "high", 50, 200);
        enq(&conn, h, w, p, "mid", 100, 150);
        let claimed = claim_ready_tasks(&conn, h, p, "owner", 600, 10, 1_000).unwrap();
        let kinds: Vec<i64> = claimed.iter().map(|c| c.id).collect();
        assert_eq!(kinds.len(), 3);
        // Verify ordering by checking the priorities of the claimed rows.
        let prios: Vec<i64> = kinds
            .iter()
            .map(|id| {
                conn.query_row(
                    "SELECT priority FROM extraction_tasks WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap()
            })
            .collect();
        assert_eq!(prios, vec![50, 100, 200]);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn claim_sets_lease_and_bumps_attempts() {
        let (conn, path, h, w, p) = fresh();
        enq(&conn, h, w, p, "k", 100, 100);
        let claimed = claim_ready_tasks(&conn, h, p, "worker-1", 600, 10, 1_000).unwrap();
        assert_eq!(claimed.len(), 1);
        let (status, owner, expires, attempts): (String, String, i64, i64) = conn
            .query_row(
                "SELECT status, lease_owner, lease_expires_epoch, attempts FROM extraction_tasks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(status, "processing");
        assert_eq!(owner, "worker-1");
        assert_eq!(expires, 1_600);
        assert_eq!(attempts, 1);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn claim_skips_rows_with_active_lease() {
        let (conn, path, h, w, p) = fresh();
        enq(&conn, h, w, p, "k", 100, 100);
        let _ = claim_ready_tasks(&conn, h, p, "owner-a", 600, 10, 1_000).unwrap();
        let second = claim_ready_tasks(&conn, h, p, "owner-b", 600, 10, 1_500).unwrap();
        assert!(second.is_empty(), "lease still active, no claim");
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn claim_ignores_delayed_until_retry_epoch() {
        let (conn, path, h, w, p) = fresh();
        let id = enq(&conn, h, w, p, "k", 100, 100);
        mark_task_delayed(&conn, id, 5_000, "transient", 1_000).unwrap();
        let early = claim_ready_tasks(&conn, h, p, "owner", 600, 10, 4_000).unwrap();
        assert!(early.is_empty(), "next_retry_epoch in the future");
        let late = claim_ready_tasks(&conn, h, p, "owner", 600, 10, 6_000).unwrap();
        assert_eq!(late.len(), 1);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn recover_expired_leases_resets_to_pending() {
        let (conn, path, h, w, p) = fresh();
        enq(&conn, h, w, p, "k", 100, 100);
        let _ = claim_ready_tasks(&conn, h, p, "stale-worker", 60, 10, 1_000).unwrap();
        // Time jumps past lease expiry.
        let n = recover_expired_leases(&conn, 5_000).unwrap();
        assert_eq!(n, 1);
        let claimed = claim_ready_tasks(&conn, h, p, "fresh-worker", 600, 10, 6_000).unwrap();
        assert_eq!(claimed.len(), 1);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn mark_task_done_clears_lease_and_advances_cursor() {
        let (conn, path, h, w, p) = fresh();
        enq(&conn, h, w, p, "k", 100, 100);
        let claimed = claim_ready_tasks(&conn, h, p, "owner", 600, 10, 1_000).unwrap();
        mark_task_done(&conn, claimed[0].id, Some(99), 2_000).unwrap();
        let (status, owner, cursor): (String, Option<String>, Option<i64>) = conn
            .query_row(
                "SELECT status, lease_owner, cursor_event_id FROM extraction_tasks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "done");
        assert!(owner.is_none());
        assert_eq!(cursor, Some(99));
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn mark_task_failed_clears_lease() {
        let (conn, path, h, w, p) = fresh();
        enq(&conn, h, w, p, "k", 100, 100);
        let claimed = claim_ready_tasks(&conn, h, p, "owner", 600, 10, 1_000).unwrap();
        mark_task_failed(&conn, claimed[0].id, "permanent", 2_000).unwrap();
        let (status, err, owner): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT status, last_error, lease_owner FROM extraction_tasks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "failed");
        assert_eq!(err.as_deref(), Some("permanent"));
        assert!(owner.is_none());
        cleanup_temp_db_files(&path);
    }
}
