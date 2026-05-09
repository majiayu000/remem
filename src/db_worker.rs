use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

pub const WORKER_HEARTBEAT_HEALTH_SECS: i64 = 480;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerHeartbeat {
    pub owner: String,
    pub pid: Option<i64>,
    pub started_at_epoch: i64,
    pub updated_at_epoch: i64,
}

pub fn upsert_worker_heartbeat(
    conn: &Connection,
    owner: &str,
    pid: i64,
    started_at_epoch: i64,
    updated_at_epoch: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO worker_heartbeats (owner, pid, started_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(owner) DO UPDATE SET
             pid = excluded.pid,
             updated_at_epoch = excluded.updated_at_epoch",
        params![owner, pid, started_at_epoch, updated_at_epoch],
    )?;
    Ok(())
}

pub fn latest_worker_heartbeat(conn: &Connection) -> Result<Option<WorkerHeartbeat>> {
    conn.query_row(
        "SELECT owner, pid, started_at_epoch, updated_at_epoch
         FROM worker_heartbeats
         ORDER BY updated_at_epoch DESC, owner ASC
         LIMIT 1",
        [],
        |row| {
            Ok(WorkerHeartbeat {
                owner: row.get(0)?,
                pid: row.get(1)?,
                started_at_epoch: row.get(2)?,
                updated_at_epoch: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn healthy_worker_heartbeat(
    conn: &Connection,
    max_age_secs: i64,
) -> Result<Option<WorkerHeartbeat>> {
    let now = chrono::Utc::now().timestamp();
    conn.query_row(
        "SELECT owner, pid, started_at_epoch, updated_at_epoch
         FROM worker_heartbeats
         WHERE updated_at_epoch >= ?1
         ORDER BY updated_at_epoch DESC, owner ASC
         LIMIT 1",
        params![now.saturating_sub(max_age_secs)],
        |row| {
            Ok(WorkerHeartbeat {
                owner: row.get(0)?,
                pid: row.get(1)?,
                started_at_epoch: row.get(2)?,
                updated_at_epoch: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{
        healthy_worker_heartbeat, latest_worker_heartbeat, upsert_worker_heartbeat,
        WORKER_HEARTBEAT_HEALTH_SECS,
    };

    fn setup(conn: &Connection) {
        conn.execute_batch(include_str!("migrations/v004_worker_heartbeat.sql"))
            .expect("heartbeat migration should run");
    }

    #[test]
    fn heartbeat_upsert_tracks_latest_healthy_worker() {
        let conn = Connection::open_in_memory().expect("db should open");
        setup(&conn);
        let now = chrono::Utc::now().timestamp();

        upsert_worker_heartbeat(&conn, "worker-old", 10, now - 900, now - 900)
            .expect("old heartbeat should insert");
        upsert_worker_heartbeat(&conn, "worker-new", 11, now - 10, now - 10)
            .expect("new heartbeat should insert");

        let latest = latest_worker_heartbeat(&conn)
            .expect("latest should load")
            .expect("heartbeat should exist");
        assert_eq!(latest.owner, "worker-new");

        let healthy = healthy_worker_heartbeat(&conn, WORKER_HEARTBEAT_HEALTH_SECS)
            .expect("healthy heartbeat should load")
            .expect("healthy heartbeat should exist");
        assert_eq!(healthy.owner, "worker-new");
    }

    #[test]
    fn stale_heartbeat_is_not_healthy() {
        let conn = Connection::open_in_memory().expect("db should open");
        setup(&conn);
        let now = chrono::Utc::now().timestamp();

        upsert_worker_heartbeat(&conn, "worker-old", 10, now - 900, now - 900)
            .expect("old heartbeat should insert");

        let healthy = healthy_worker_heartbeat(&conn, WORKER_HEARTBEAT_HEALTH_SECS)
            .expect("healthy heartbeat query should run");
        assert!(healthy.is_none());
    }
}
