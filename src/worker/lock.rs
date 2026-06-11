use std::fs::{File, OpenOptions};
use std::path::PathBuf;

use anyhow::{Context, Result};
use fs2::FileExt;

pub(super) struct WorkerLockGuard {
    file: File,
}

pub(super) fn acquire_worker_singleton() -> Result<Option<WorkerLockGuard>> {
    let path = worker_lock_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create worker lock directory {}", parent.display()))?;
    }

    acquire_file_lock(path)
}

pub(super) fn worker_lock_path() -> Result<PathBuf> {
    Ok(crate::db::absolute_data_dir()?.join("worker.lock"))
}

fn acquire_file_lock(path: PathBuf) -> Result<Option<WorkerLockGuard>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("open worker lock {}", path.display()))?;

    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(WorkerLockGuard { file })),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("lock worker singleton {}", path.display()))
        }
    }
}

impl Drop for WorkerLockGuard {
    fn drop(&mut self) {
        if let Err(error) = self.file.unlock() {
            crate::log::warn(
                "worker",
                &format!("unlock worker singleton failed: {}", error),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
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
        assert_eq!(worker_lock_path()?, data_dir.path.join("worker.lock"));

        drop(first);
        assert!(acquire_worker_singleton()?.is_some());
        Ok(())
    }

    #[test]
    fn worker_lock_path_absolutizes_relative_data_dir() -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("worker-lock-relative-data-dir");
        let relative = PathBuf::from(format!(
            ".remem-worker-lock-relative-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        if relative.exists() {
            std::fs::remove_dir_all(&relative)?;
        }
        std::env::set_var("REMEM_DATA_DIR", &relative);
        let expected = std::env::current_dir()?.join(&relative).join("worker.lock");

        assert_eq!(worker_lock_path()?, expected);
        if relative.exists() {
            std::fs::remove_dir_all(relative)?;
        }
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
