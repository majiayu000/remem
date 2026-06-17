use std::fs::{File, OpenOptions};
use std::path::PathBuf;

use anyhow::{Context, Result};
use fs2::FileExt;

use crate::db;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkerSpawnDecision {
    Spawned,
    SkippedHealthyDaemon,
    SkippedLaunchInProgress,
}

pub(super) fn spawn_worker_once_if_idle(
    conn: &rusqlite::Connection,
) -> Result<WorkerSpawnDecision> {
    spawn_worker_once_if_idle_with(conn, spawn_worker_once)
}

fn spawn_worker_once_if_idle_with(
    conn: &rusqlite::Connection,
    spawn: impl FnOnce() -> Result<()>,
) -> Result<WorkerSpawnDecision> {
    if !should_spawn_worker_once(conn)? {
        return Ok(WorkerSpawnDecision::SkippedHealthyDaemon);
    }
    let Some(_guard) = acquire_worker_launch_lock()? else {
        return Ok(WorkerSpawnDecision::SkippedLaunchInProgress);
    };
    if !should_spawn_worker_once(conn)? {
        return Ok(WorkerSpawnDecision::SkippedHealthyDaemon);
    }
    spawn()?;
    Ok(WorkerSpawnDecision::Spawned)
}

fn should_spawn_worker_once(conn: &rusqlite::Connection) -> Result<bool> {
    Ok(db::healthy_daemon_worker_heartbeat(conn, db::WORKER_HEARTBEAT_HEALTH_SECS)?.is_none())
}

fn spawn_worker_once() -> Result<()> {
    let exe = std::env::current_exe()?;
    let worker_dir = stable_worker_dir();
    let stderr_file = crate::log::open_log_append();
    let stderr_cfg = match stderr_file {
        Some(file) => std::process::Stdio::from(file),
        None => std::process::Stdio::null(),
    };
    let mut command = std::process::Command::new(&exe);
    command
        .arg("worker")
        .arg("--once")
        .current_dir(&worker_dir)
        .env("REMEM_DATA_DIR", &worker_dir)
        .env("REMEM_STDERR_TO_LOG", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr_cfg);
    let _child = command.spawn()?;
    Ok(())
}

fn stable_worker_dir() -> PathBuf {
    let data_dir = match crate::db::absolute_data_dir() {
        Ok(path) => path,
        Err(err) => {
            crate::log::warn(
                "summarize",
                &format!(
                    "failed to resolve worker dir from REMEM_DATA_DIR: {}; falling back to temp dir",
                    err
                ),
            );
            return std::env::temp_dir();
        }
    };
    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        crate::log::warn(
            "summarize",
            &format!(
                "failed to create worker dir {}: {}; falling back to temp dir",
                data_dir.display(),
                err
            ),
        );
        return std::env::temp_dir();
    }
    data_dir
}

struct WorkerLaunchLockGuard {
    file: File,
}

fn acquire_worker_launch_lock() -> Result<Option<WorkerLaunchLockGuard>> {
    let path = worker_launch_lock_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create worker launch lock directory {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("open worker launch lock {}", path.display()))?;

    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(WorkerLaunchLockGuard { file })),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(error) => Err(error).with_context(|| format!("lock worker launch {}", path.display())),
    }
}

fn worker_launch_lock_path() -> Result<PathBuf> {
    Ok(crate::db::absolute_data_dir()?.join("worker-launch.lock"))
}

impl Drop for WorkerLaunchLockGuard {
    fn drop(&mut self) {
        if let Err(error) = self.file.unlock() {
            crate::log::warn(
                "summarize",
                &format!("unlock worker launch failed: {}", error),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    };
    use std::time::Duration;

    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::{
        should_spawn_worker_once, spawn_worker_once_if_idle_with, stable_worker_dir,
        WorkerSpawnDecision,
    };

    #[test]
    fn missing_worker_uses_stop_fallback_spawn() {
        let _test_dir = ScopedTestDataDir::new("summary-missing-worker");
        let conn = db::open_db().expect("db should open");

        assert!(
            should_spawn_worker_once(&conn).expect("worker check should run"),
            "missing heartbeat should keep worker --once fallback"
        );
    }

    #[test]
    fn healthy_daemon_skips_stop_spawn() {
        let _test_dir = ScopedTestDataDir::new("summary-healthy-daemon");
        let conn = db::open_db().expect("db should open");
        let now = chrono::Utc::now().timestamp();
        db::upsert_worker_heartbeat(
            &conn,
            "worker-daemon",
            i64::from(std::process::id()),
            now - 5,
            now - 5,
        )
        .expect("heartbeat should insert");

        assert!(
            !should_spawn_worker_once(&conn).expect("worker check should run"),
            "healthy daemon heartbeat should skip worker --once fallback"
        );
    }

    #[test]
    fn healthy_once_worker_does_not_skip_stop_spawn() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("summary-healthy-once-worker");
        let conn = db::open_db()?;
        let now = chrono::Utc::now().timestamp();
        db::upsert_worker_heartbeat(
            &conn,
            "worker-once-test",
            i64::from(std::process::id()),
            now - 5,
            now - 5,
        )?;

        assert!(
            should_spawn_worker_once(&conn)?,
            "healthy worker --once heartbeat should not suppress Stop fallback"
        );
        Ok(())
    }

    #[test]
    fn stale_worker_uses_stop_fallback_spawn() {
        let _test_dir = ScopedTestDataDir::new("summary-stale-worker");
        let conn = db::open_db().expect("db should open");
        let now = chrono::Utc::now().timestamp();
        db::upsert_worker_heartbeat(&conn, "worker-once-old", 123, now - 900, now - 900)
            .expect("heartbeat should insert");

        assert!(
            should_spawn_worker_once(&conn).expect("worker check should run"),
            "stale heartbeat should keep worker --once fallback"
        );
    }

    #[test]
    fn concurrent_stop_spawn_attempts_are_bounded_by_launch_lock() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("summary-launch-lock-concurrent");
        let setup = db::open_db()?;
        drop(setup);

        let workers = 8;
        let barrier = Arc::new(Barrier::new(workers));
        let spawned = Arc::new(AtomicUsize::new(0));
        let skipped_launch = Arc::new(AtomicUsize::new(0));
        let skipped_healthy = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..workers {
                let barrier = Arc::clone(&barrier);
                let spawned = Arc::clone(&spawned);
                let skipped_launch = Arc::clone(&skipped_launch);
                let skipped_healthy = Arc::clone(&skipped_healthy);
                handles.push(scope.spawn(move || -> anyhow::Result<()> {
                    let conn = db::open_db()?;
                    barrier.wait();
                    let decision = spawn_worker_once_if_idle_with(&conn, || {
                        spawned.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(50));
                        Ok(())
                    })?;
                    match decision {
                        WorkerSpawnDecision::Spawned => {}
                        WorkerSpawnDecision::SkippedLaunchInProgress => {
                            skipped_launch.fetch_add(1, Ordering::SeqCst);
                        }
                        WorkerSpawnDecision::SkippedHealthyDaemon => {
                            skipped_healthy.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    Ok(())
                }));
            }
            for handle in handles {
                handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("spawn thread panicked"))??;
            }
            Ok::<(), anyhow::Error>(())
        })?;

        assert_eq!(spawned.load(Ordering::SeqCst), 1);
        assert_eq!(
            spawned.load(Ordering::SeqCst)
                + skipped_launch.load(Ordering::SeqCst)
                + skipped_healthy.load(Ordering::SeqCst),
            workers
        );
        Ok(())
    }

    #[test]
    fn stable_worker_dir_uses_data_dir() {
        let data_dir = ScopedTestDataDir::new("summary-worker-dir");

        let got = stable_worker_dir();

        assert_eq!(got, data_dir.path);
        assert!(got.is_dir());
    }

    #[test]
    fn stable_worker_dir_absolutizes_relative_data_dir() -> anyhow::Result<()> {
        let relative = std::path::PathBuf::from(format!(
            ".remem-summary-worker-relative-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));

        let got = db::with_data_dir(&relative, stable_worker_dir);

        assert_eq!(got, std::env::current_dir()?.join(&relative));
        assert!(got.is_absolute());
        assert!(got.is_dir());
        std::fs::remove_dir_all(relative)?;
        Ok(())
    }
}
