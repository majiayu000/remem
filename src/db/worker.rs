use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

pub const WORKER_HEARTBEAT_HEALTH_SECS: i64 = 480;
const CURRENT_WORKER_OWNER_PREFIX: &str = concat!("worker-v", env!("CARGO_PKG_VERSION"), "-");

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

pub fn current_worker_owner(mode: &str, pid: u32, epoch_millis: i64) -> String {
    format!("{CURRENT_WORKER_OWNER_PREFIX}{mode}-{pid}-{epoch_millis}")
}

pub(crate) fn exact_replay_worker_owner(pid: u32, epoch_millis: i64) -> String {
    current_worker_owner("exact-replay", pid, epoch_millis)
}

pub(crate) fn is_exact_replay_worker_owner(owner: &str) -> bool {
    let Some((version, suffix)) = owner
        .strip_prefix("worker-v")
        .and_then(|value| value.split_once("-exact-replay-"))
    else {
        return false;
    };
    let Some((pid, epoch_millis)) = suffix.split_once('-') else {
        return false;
    };
    !version.is_empty() && pid.parse::<u32>().is_ok() && epoch_millis.parse::<i64>().is_ok()
}

pub fn is_current_daemon_worker_owner(owner: &str) -> bool {
    owner.starts_with(&format!("{CURRENT_WORKER_OWNER_PREFIX}daemon-"))
}

pub fn latest_worker_heartbeat(conn: &Connection) -> Result<Option<WorkerHeartbeat>> {
    query_latest_worker_heartbeat(conn, false)
}

pub fn latest_daemon_worker_heartbeat(conn: &Connection) -> Result<Option<WorkerHeartbeat>> {
    query_latest_worker_heartbeat(conn, true)
}

fn query_latest_worker_heartbeat(
    conn: &Connection,
    daemon_only: bool,
) -> Result<Option<WorkerHeartbeat>> {
    let daemon_filter = if daemon_only {
        "WHERE owner NOT LIKE 'worker-once-%'
           AND owner NOT LIKE 'worker-v%-once-%'"
    } else {
        ""
    };
    conn.query_row(
        &format!(
            "SELECT owner, pid, started_at_epoch, updated_at_epoch
         FROM worker_heartbeats
         {daemon_filter}
         ORDER BY updated_at_epoch DESC, owner ASC
         LIMIT 1"
        ),
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
    query_healthy_worker_heartbeat(conn, max_age_secs, false, None)
}

pub fn healthy_daemon_worker_heartbeat(
    conn: &Connection,
    max_age_secs: i64,
) -> Result<Option<WorkerHeartbeat>> {
    query_healthy_worker_heartbeat(conn, max_age_secs, true, None)
}

pub(crate) fn healthy_current_once_worker_heartbeat(
    conn: &Connection,
    max_age_secs: i64,
) -> Result<Option<WorkerHeartbeat>> {
    let owner_pattern = format!("{CURRENT_WORKER_OWNER_PREFIX}once-%");
    query_healthy_worker_heartbeat(conn, max_age_secs, false, Some(&owner_pattern))
}

pub(crate) fn healthy_exact_replay_worker_heartbeat(
    conn: &Connection,
    max_age_secs: i64,
) -> Result<Option<WorkerHeartbeat>> {
    query_healthy_worker_heartbeat(conn, max_age_secs, false, Some("worker-v%-exact-replay-%"))
}

fn query_healthy_worker_heartbeat(
    conn: &Connection,
    max_age_secs: i64,
    daemon_only: bool,
    owner_pattern: Option<&str>,
) -> Result<Option<WorkerHeartbeat>> {
    let now = chrono::Utc::now().timestamp();
    let daemon_filter = if daemon_only {
        " AND owner NOT LIKE 'worker-once-%'
          AND owner NOT LIKE 'worker-v%-once-%'"
    } else {
        ""
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT owner, pid, started_at_epoch, updated_at_epoch
         FROM worker_heartbeats
         WHERE updated_at_epoch >= ?1
         {daemon_filter}
           AND (?2 IS NULL OR owner LIKE ?2)
         ORDER BY updated_at_epoch DESC, owner ASC
         LIMIT 10"
    ))?;
    let rows = stmt.query_map(
        params![now.saturating_sub(max_age_secs), owner_pattern],
        |row| {
            Ok(WorkerHeartbeat {
                owner: row.get(0)?,
                pid: row.get(1)?,
                started_at_epoch: row.get(2)?,
                updated_at_epoch: row.get(3)?,
            })
        },
    )?;

    for row in rows {
        let heartbeat = row?;
        if heartbeat_process_alive(heartbeat.pid) {
            return Ok(Some(heartbeat));
        }
    }
    Ok(None)
}

fn heartbeat_process_alive(pid: Option<i64>) -> bool {
    let Some(pid) = pid else {
        return false;
    };
    if pid <= 0 || pid > i64::from(i32::MAX) {
        return false;
    }

    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if result == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
pub(crate) fn test_heartbeat_process_alive(pid: Option<i64>) -> bool {
    heartbeat_process_alive(pid)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{
        current_worker_owner, healthy_daemon_worker_heartbeat, healthy_worker_heartbeat,
        is_current_daemon_worker_owner, is_exact_replay_worker_owner,
        latest_daemon_worker_heartbeat, latest_worker_heartbeat, test_heartbeat_process_alive,
        upsert_worker_heartbeat, WORKER_HEARTBEAT_HEALTH_SECS,
    };

    fn setup(conn: &Connection) {
        conn.execute_batch(include_str!("../migrations/v004_worker_heartbeat.sql"))
            .expect("heartbeat migration should run");
    }

    #[test]
    fn heartbeat_upsert_tracks_latest_healthy_worker() {
        let conn = Connection::open_in_memory().expect("db should open");
        setup(&conn);
        let now = chrono::Utc::now().timestamp();

        let current_pid = i64::from(std::process::id());

        upsert_worker_heartbeat(&conn, "worker-old", current_pid, now - 900, now - 900)
            .expect("old heartbeat should insert");
        upsert_worker_heartbeat(&conn, "worker-new", current_pid, now - 10, now - 10)
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
    fn current_worker_owner_is_version_gated() {
        let owner = current_worker_owner("daemon", 123, 456);

        assert!(is_current_daemon_worker_owner(&owner));
        assert!(!is_current_daemon_worker_owner("worker-daemon-123-456"));
        assert!(!is_current_daemon_worker_owner(&current_worker_owner(
            "once", 123, 456
        )));
    }

    #[test]
    fn exact_replay_worker_owner_is_stable_across_versions() {
        assert!(is_exact_replay_worker_owner(
            "worker-v0.5.9-exact-replay-123-456"
        ));
        assert!(is_exact_replay_worker_owner(
            "worker-v0.7.0-beta.1-exact-replay-123-456"
        ));
        assert!(!is_exact_replay_worker_owner(
            "worker-v0.5.9-daemon-123-456"
        ));
        assert!(!is_exact_replay_worker_owner(
            "worker-v0.5.9-exact-replay-invalid-456"
        ));
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

    #[test]
    fn recent_heartbeat_with_dead_pid_is_not_healthy() {
        let conn = Connection::open_in_memory().expect("db should open");
        setup(&conn);
        let now = chrono::Utc::now().timestamp();

        upsert_worker_heartbeat(&conn, "worker-dead", i64::from(i32::MAX), now, now)
            .expect("dead heartbeat should insert");

        let healthy = healthy_worker_heartbeat(&conn, WORKER_HEARTBEAT_HEALTH_SECS)
            .expect("healthy heartbeat query should run");
        assert!(healthy.is_none());
    }

    #[test]
    fn once_heartbeat_is_not_daemon_healthy() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        setup(&conn);
        let now = chrono::Utc::now().timestamp();

        upsert_worker_heartbeat(
            &conn,
            "worker-once-test",
            i64::from(std::process::id()),
            now,
            now,
        )?;
        let current_once_owner = current_worker_owner("once", std::process::id(), now * 1000);
        upsert_worker_heartbeat(
            &conn,
            &current_once_owner,
            i64::from(std::process::id()),
            now + 1,
            now + 1,
        )?;

        let healthy = healthy_worker_heartbeat(&conn, WORKER_HEARTBEAT_HEALTH_SECS)?;
        assert_eq!(
            healthy.as_ref().map(|heartbeat| heartbeat.owner.as_str()),
            Some(current_once_owner.as_str())
        );
        let healthy_daemon = healthy_daemon_worker_heartbeat(&conn, WORKER_HEARTBEAT_HEALTH_SECS)?;
        assert!(healthy_daemon.is_none());
        Ok(())
    }

    #[test]
    fn legacy_worker_heartbeat_counts_as_daemon_healthy() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        setup(&conn);
        let now = chrono::Utc::now().timestamp();

        upsert_worker_heartbeat(
            &conn,
            "worker-legacy",
            i64::from(std::process::id()),
            now,
            now,
        )?;

        let healthy_daemon = healthy_daemon_worker_heartbeat(&conn, WORKER_HEARTBEAT_HEALTH_SECS)?;
        let Some(healthy_daemon) = healthy_daemon else {
            anyhow::bail!("legacy daemon heartbeat should be healthy");
        };
        assert_eq!(healthy_daemon.owner, "worker-legacy");
        Ok(())
    }

    #[test]
    fn latest_daemon_heartbeat_ignores_once_heartbeat() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        setup(&conn);
        let now = chrono::Utc::now().timestamp();

        upsert_worker_heartbeat(
            &conn,
            "worker-daemon-test",
            i64::from(std::process::id()),
            now - 10,
            now - 10,
        )?;
        upsert_worker_heartbeat(
            &conn,
            "worker-once-test",
            i64::from(std::process::id()),
            now,
            now,
        )?;
        let current_once_owner = current_worker_owner("once", std::process::id(), now * 1000);
        upsert_worker_heartbeat(
            &conn,
            &current_once_owner,
            i64::from(std::process::id()),
            now + 1,
            now + 1,
        )?;

        let latest = latest_worker_heartbeat(&conn)?;
        assert_eq!(
            latest.as_ref().map(|heartbeat| heartbeat.owner.as_str()),
            Some(current_once_owner.as_str())
        );

        let latest_daemon = latest_daemon_worker_heartbeat(&conn)?;
        assert_eq!(
            latest_daemon
                .as_ref()
                .map(|heartbeat| heartbeat.owner.as_str()),
            Some("worker-daemon-test")
        );
        Ok(())
    }

    #[test]
    fn current_process_pid_is_alive() {
        assert!(test_heartbeat_process_alive(Some(i64::from(
            std::process::id()
        ))));
    }
}
