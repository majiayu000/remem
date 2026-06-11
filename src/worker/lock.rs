use std::fs::{File, OpenOptions};
use std::path::PathBuf;

use anyhow::{Context, Result};

pub(super) struct WorkerLockGuard {
    file: File,
    #[cfg(not(unix))]
    path: PathBuf,
}

pub(super) fn acquire_worker_singleton() -> Result<Option<WorkerLockGuard>> {
    let path = worker_lock_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create worker lock directory {}", parent.display()))?;
    }

    #[cfg(unix)]
    {
        acquire_unix_lock(path)
    }

    #[cfg(not(unix))]
    {
        acquire_fallback_lock(path)
    }
}

pub(super) fn worker_lock_path() -> PathBuf {
    crate::db::data_dir().join("worker.lock")
}

#[cfg(unix)]
fn acquire_unix_lock(path: PathBuf) -> Result<Option<WorkerLockGuard>> {
    use std::os::fd::AsRawFd;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("open worker lock {}", path.display()))?;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(Some(WorkerLockGuard { file }));
    }

    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN => Ok(None),
        _ => Err(error).with_context(|| format!("lock worker singleton {}", path.display())),
    }
}

#[cfg(not(unix))]
fn acquire_fallback_lock(path: PathBuf) -> Result<Option<WorkerLockGuard>> {
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => Ok(Some(WorkerLockGuard { file, path })),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(error) => Err(error).with_context(|| format!("create worker lock {}", path.display())),
    }
}

impl Drop for WorkerLockGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;

            let result = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                crate::log::warn(
                    "worker",
                    &format!("unlock worker singleton failed: {}", error),
                );
            }
        }

        #[cfg(not(unix))]
        {
            if let Err(error) = std::fs::remove_file(&self.path) {
                crate::log::warn(
                    "worker",
                    &format!("remove worker singleton lock failed: {}", error),
                );
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    };
    use std::time::Duration;

    use crate::db::test_support::ScopedTestDataDir;

    use super::{acquire_worker_singleton, worker_lock_path};

    #[test]
    fn singleton_returns_none_when_lock_is_held() -> anyhow::Result<()> {
        let data_dir = ScopedTestDataDir::new("worker-lock-held");
        let Some(first) = acquire_worker_singleton()? else {
            anyhow::bail!("first worker lock should acquire");
        };

        assert!(acquire_worker_singleton()?.is_none());
        assert_eq!(worker_lock_path(), data_dir.path.join("worker.lock"));

        drop(first);
        assert!(acquire_worker_singleton()?.is_some());
        Ok(())
    }

    #[test]
    fn singleton_excludes_concurrent_acquire() -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("worker-lock-concurrent");
        let barrier = Arc::new(Barrier::new(2));
        let acquired = Arc::new(AtomicUsize::new(0));
        let blocked = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..2 {
                let barrier = Arc::clone(&barrier);
                let acquired = Arc::clone(&acquired);
                let blocked = Arc::clone(&blocked);
                handles.push(scope.spawn(move || -> anyhow::Result<()> {
                    barrier.wait();
                    match acquire_worker_singleton()? {
                        Some(_guard) => {
                            acquired.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        None => {
                            blocked.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    Ok(())
                }));
            }
            for handle in handles {
                handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("lock thread panicked"))??;
            }
            Ok::<(), anyhow::Error>(())
        })?;

        assert_eq!(acquired.load(Ordering::SeqCst), 1);
        assert_eq!(blocked.load(Ordering::SeqCst), 1);
        Ok(())
    }
}
