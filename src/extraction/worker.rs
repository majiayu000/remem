use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use tokio::time::{sleep, Duration};

use crate::extraction::{
    claim_next_ready_task, mark_task_delayed, mark_task_done, mark_task_failed,
    recover_expired_leases, ClaimedTask, TaskKind, DEFAULT_LEASE_SECS,
};

const TASK_TIMEOUT_SECS: u64 = 420;
const TASK_LEASE_SECS: i64 = DEFAULT_LEASE_SECS;
const MAX_TASK_ATTEMPTS: i64 = 5;
pub const WORKER_HEARTBEAT_HEALTH_SECS: i64 = 480;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerHeartbeat {
    pub owner: String,
    pub pid: Option<i64>,
    pub mode: String,
    pub started_at_epoch: i64,
    pub updated_at_epoch: i64,
}

enum TaskOutcome {
    #[allow(dead_code)]
    Done {
        new_cursor: Option<i64>,
    },
    PermanentFailure(String),
}

fn retry_backoff_secs(attempt: i64) -> i64 {
    match attempt {
        0 | 1 => 5,
        2 => 15,
        3 => 45,
        4 => 120,
        _ => 300,
    }
}

fn record_worker_heartbeat(
    conn: &Connection,
    owner: &str,
    mode: &str,
    started_at_epoch: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO worker_heartbeats(owner, pid, mode, started_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(owner) DO UPDATE SET
             pid = excluded.pid,
             mode = excluded.mode,
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            owner,
            i64::from(std::process::id()),
            mode,
            started_at_epoch,
            now
        ],
    )?;
    Ok(())
}

pub fn latest_worker_heartbeat(conn: &Connection) -> Result<Option<WorkerHeartbeat>> {
    conn.query_row(
        "SELECT owner, pid, mode, started_at_epoch, updated_at_epoch
         FROM worker_heartbeats
         ORDER BY updated_at_epoch DESC, owner ASC
         LIMIT 1",
        [],
        |row| {
            Ok(WorkerHeartbeat {
                owner: row.get(0)?,
                pid: row.get(1)?,
                mode: row.get(2)?,
                started_at_epoch: row.get(3)?,
                updated_at_epoch: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn healthy_worker_heartbeat(conn: &Connection) -> Result<Option<WorkerHeartbeat>> {
    let now = chrono::Utc::now().timestamp();
    conn.query_row(
        "SELECT owner, pid, mode, started_at_epoch, updated_at_epoch
         FROM worker_heartbeats
         WHERE mode = 'daemon' AND updated_at_epoch >= ?1
         ORDER BY updated_at_epoch DESC, owner ASC
         LIMIT 1",
        params![now.saturating_sub(WORKER_HEARTBEAT_HEALTH_SECS)],
        |row| {
            Ok(WorkerHeartbeat {
                owner: row.get(0)?,
                pid: row.get(1)?,
                mode: row.get(2)?,
                started_at_epoch: row.get(3)?,
                updated_at_epoch: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

async fn process_task(task: &ClaimedTask) -> Result<TaskOutcome> {
    let message = match task.task_kind {
        TaskKind::SessionRollup => "session_rollup handler is not implemented",
        TaskKind::ObservationExtract => "observation_extract handler is not implemented",
        TaskKind::MemoryCandidate => "memory_candidate handler is not implemented",
        TaskKind::RuleCandidate => "rule_candidate handler is not implemented",
        TaskKind::IndexUpdate => "index_update handler is not implemented",
    };
    Ok(TaskOutcome::PermanentFailure(message.to_string()))
}

fn fail_or_delay_task(
    conn: &Connection,
    task: &ClaimedTask,
    message: &str,
    now: i64,
) -> Result<()> {
    if task.attempts >= MAX_TASK_ATTEMPTS {
        mark_task_failed(conn, task.id, message, now)
    } else {
        mark_task_delayed(
            conn,
            task.id,
            now + retry_backoff_secs(task.attempts),
            message,
            now,
        )
    }
}

pub async fn run(once: bool, idle_sleep_ms: u64) -> Result<()> {
    let started_at_epoch = chrono::Utc::now().timestamp();
    let owner = format!(
        "worker-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );
    let mode = if once { "once" } else { "daemon" };
    crate::log::info("worker", &format!("start owner={} mode={}", owner, mode));

    loop {
        let conn = crate::db::schema::open()?;
        record_worker_heartbeat(&conn, &owner, mode, started_at_epoch)?;

        let now = chrono::Utc::now().timestamp();
        let recovered = recover_expired_leases(&conn, now)?;
        if recovered > 0 {
            crate::log::warn(
                "worker",
                &format!("recovered {} expired extraction task lease(s)", recovered),
            );
        }

        let Some(task) = claim_next_ready_task(&conn, &owner, TASK_LEASE_SECS, now)? else {
            if once {
                break;
            }
            sleep(Duration::from_millis(idle_sleep_ms.max(100))).await;
            continue;
        };

        crate::log::info(
            "worker",
            &format!(
                "claimed extraction task id={} kind={} attempt={}",
                task.id,
                task.task_kind.as_db_value(),
                task.attempts
            ),
        );

        let timed =
            tokio::time::timeout(Duration::from_secs(TASK_TIMEOUT_SECS), process_task(&task)).await;
        let conn = crate::db::schema::open()?;
        let now = chrono::Utc::now().timestamp();
        match timed {
            Ok(Ok(TaskOutcome::Done { new_cursor })) => {
                mark_task_done(&conn, task.id, new_cursor, now)?;
                crate::log::info("worker", &format!("done extraction task id={}", task.id));
            }
            Ok(Ok(TaskOutcome::PermanentFailure(message))) => {
                mark_task_failed(&conn, task.id, &message, now)?;
                crate::log::warn(
                    "worker",
                    &format!("extraction task id={} failed: {}", task.id, message),
                );
            }
            Ok(Err(error)) => {
                let message = error.to_string();
                fail_or_delay_task(&conn, &task, &message, now)?;
                crate::log::warn(
                    "worker",
                    &format!("extraction task id={} error: {}", task.id, message),
                );
            }
            Err(_) => {
                let message = format!("task timed out after {}s", TASK_TIMEOUT_SECS);
                fail_or_delay_task(&conn, &task, &message, now)?;
                crate::log::warn("worker", &format!("extraction task id={} timeout", task.id));
            }
        }
    }

    let conn = crate::db::schema::open()?;
    record_worker_heartbeat(&conn, &owner, mode, started_at_epoch)?;
    crate::log::info("worker", "stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::{params, Connection};

    use super::*;
    use crate::db::test_support::ScopedTestDataDir;
    use crate::extraction::{enqueue_extraction_task, EnqueueRequest};

    fn seed_identity(conn: &Connection) -> (i64, i64, i64) {
        conn.execute(
            "INSERT INTO workspaces(root_path, created_at_epoch, updated_at_epoch)
             VALUES ('/tmp/repo', 0, 0)",
            [],
        )
        .unwrap();
        let workspace_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO projects(workspace_id, project_path, project_key,
                created_at_epoch, updated_at_epoch)
             VALUES (?1, '/tmp/repo', '/tmp/repo', 0, 0)",
            [workspace_id],
        )
        .unwrap();
        let project_id = conn.last_insert_rowid();
        let host_id: i64 = conn
            .query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |row| {
                row.get(0)
            })
            .unwrap();
        (host_id, workspace_id, project_id)
    }

    fn enqueue_task(conn: &Connection, task_kind: TaskKind) -> i64 {
        let (host_id, workspace_id, project_id) = seed_identity(conn);
        enqueue_extraction_task(
            conn,
            EnqueueRequest {
                task_kind,
                host_id,
                workspace_id,
                project_id,
                session_row_id: None,
                priority: 100,
                idempotency_key: task_kind.as_db_value(),
                high_watermark_event_id: Some(7),
                now: 1_000,
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn worker_marks_unimplemented_task_failed() -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("extraction-worker-unimplemented");
        let conn = crate::db::schema::open()?;
        let task_id = enqueue_task(&conn, TaskKind::SessionRollup);
        drop(conn);

        run(true, 10).await?;

        let conn = crate::db::schema::open()?;
        let (status, last_error, owner): (String, String, Option<String>) = conn.query_row(
            "SELECT status, last_error, lease_owner FROM extraction_tasks WHERE id = ?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(status, "failed");
        assert!(last_error.contains("handler is not implemented"));
        assert!(owner.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn worker_recovers_expired_lease_before_claiming() -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("extraction-worker-recover");
        let conn = crate::db::schema::open()?;
        let task_id = enqueue_task(&conn, TaskKind::ObservationExtract);
        conn.execute(
            "UPDATE extraction_tasks
             SET status = 'processing',
                 lease_owner = 'old-worker',
                 lease_expires_epoch = ?1
             WHERE id = ?2",
            params![chrono::Utc::now().timestamp() - 60, task_id],
        )?;
        drop(conn);

        run(true, 10).await?;

        let conn = crate::db::schema::open()?;
        let (status, attempts): (String, i64) = conn.query_row(
            "SELECT status, attempts FROM extraction_tasks WHERE id = ?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(status, "failed");
        assert_eq!(attempts, 1);
        Ok(())
    }

    #[tokio::test]
    async fn daemon_records_healthy_heartbeat() -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("extraction-worker-heartbeat");
        let timed =
            tokio::time::timeout(std::time::Duration::from_millis(40), run(false, 10)).await;
        assert!(timed.is_err(), "daemon worker should keep running");

        let conn = crate::db::schema::open()?;
        let heartbeat = healthy_worker_heartbeat(&conn)?.expect("heartbeat should be healthy");
        assert_eq!(heartbeat.mode, "daemon");
        assert!(heartbeat.owner.starts_with("worker-"));
        assert!(heartbeat.updated_at_epoch >= heartbeat.started_at_epoch);
        Ok(())
    }
}
