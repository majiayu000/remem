use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use fs2::FileExt;

static CLAIM_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub(crate) struct SpillQueue {
    active_path: PathBuf,
    stem: String,
}

#[derive(Debug)]
pub(crate) struct SpillClaim {
    queue: SpillQueue,
    claimed_path: PathBuf,
    failed_path: PathBuf,
}

impl SpillQueue {
    pub(crate) fn new(active_path: PathBuf) -> Result<Self> {
        let stem = active_path
            .file_stem()
            .and_then(|value| value.to_str())
            .context("spill queue path requires a UTF-8 file stem")?
            .to_string();
        Ok(Self { active_path, stem })
    }

    pub(crate) fn append_line(&self, line: &[u8]) -> Result<()> {
        let mut contents = line.to_vec();
        contents.push(b'\n');
        self.append_bytes(&contents)
    }

    pub(crate) fn append_bytes(&self, contents: &[u8]) -> Result<()> {
        if contents.is_empty() {
            return Ok(());
        }
        self.with_lock(|| self.append_bytes_unlocked(contents))
    }

    pub(crate) fn claim(&self, orphan_age: Duration) -> Result<Option<SpillClaim>> {
        self.restore_orphaned_claims(orphan_age)?;
        let claimed_path = self.next_claim_path();
        let rename = self.with_lock(|| {
            std::fs::rename(&self.active_path, &claimed_path).with_context(|| {
                format!(
                    "claim spill queue {} as {}",
                    self.active_path.display(),
                    claimed_path.display()
                )
            })
        });
        match rename {
            Ok(()) => Ok(Some(SpillClaim {
                queue: self.clone(),
                failed_path: claimed_path.with_extension("failed.jsonl"),
                claimed_path,
            })),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == ErrorKind::NotFound) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn restore_orphaned_claims(&self, min_age: Duration) -> Result<usize> {
        let dir = self
            .active_path
            .parent()
            .context("spill queue path requires a parent directory")?;
        self.with_lock(|| {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
                Err(error) => return Err(error).with_context(|| format!("read {}", dir.display())),
            };
            let mut restored = 0;
            for entry in entries {
                let claimed_path = entry?.path();
                if !self.is_claim_path(&claimed_path)
                    || !is_stale_claim(&claimed_path, &self.stem, min_age)
                {
                    continue;
                }
                let contents = std::fs::read(&claimed_path)
                    .with_context(|| format!("read {}", claimed_path.display()))?;
                self.append_bytes_unlocked(&contents)?;
                remove_file_if_exists(&claimed_path)?;
                remove_file_if_exists(&claimed_path.with_extension("failed.jsonl"))?;
                restored += 1;
            }
            Ok(restored)
        })
    }

    fn append_bytes_unlocked(&self, contents: &[u8]) -> Result<()> {
        if let Some(parent) = self.active_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create spill queue dir {}", parent.display()))?;
        }
        let mut file = append_open_options()
            .open(&self.active_path)
            .with_context(|| format!("open spill queue {}", self.active_path.display()))?;
        file.write_all(contents)
            .with_context(|| format!("append spill queue {}", self.active_path.display()))
    }

    fn append_file_then_remove(&self, records_path: &Path) -> Result<()> {
        let contents = std::fs::read(records_path)
            .with_context(|| format!("read {}", records_path.display()))?;
        self.with_lock(|| {
            self.append_bytes_unlocked(&contents)?;
            remove_file_if_exists(records_path)
        })
    }

    fn with_lock<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        let lock_path = self.active_path.with_extension("lock");
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create spill lock dir {}", parent.display()))?;
        }
        let lock_file = append_open_options()
            .open(&lock_path)
            .with_context(|| format!("open spill lock {}", lock_path.display()))?;
        lock_file
            .lock_exclusive()
            .with_context(|| format!("lock spill queue {}", lock_path.display()))?;
        let result = operation();
        let unlock = lock_file
            .unlock()
            .with_context(|| format!("unlock spill queue {}", lock_path.display()));
        match (result, unlock) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(unlock_error)) => Err(error.context(unlock_error.to_string())),
        }
    }

    pub(crate) fn next_claim_path(&self) -> PathBuf {
        let sequence = CLAIM_COUNTER.fetch_add(1, Ordering::Relaxed);
        let now_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        self.active_path.with_file_name(format!(
            "{}.replay-{}-{now_nanos}-{sequence}.jsonl",
            self.stem,
            std::process::id()
        ))
    }

    #[cfg(test)]
    pub(crate) fn adopt_claim(&self, claimed_path: PathBuf) -> SpillClaim {
        SpillClaim {
            queue: self.clone(),
            failed_path: claimed_path.with_extension("failed.jsonl"),
            claimed_path,
        }
    }

    fn is_claim_path(&self, path: &Path) -> bool {
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            return false;
        };
        file_name.starts_with(&format!("{}.replay-", self.stem))
            && file_name.ends_with(".jsonl")
            && !file_name.ends_with(".failed.jsonl")
    }
}

impl SpillClaim {
    pub(crate) fn path(&self) -> &Path {
        &self.claimed_path
    }

    pub(crate) fn failed_path(&self) -> &Path {
        &self.failed_path
    }

    pub(crate) fn finish(&self) -> Result<()> {
        if self.failed_path.exists() {
            self.queue.append_file_then_remove(&self.failed_path)?;
        }
        remove_file_if_exists(&self.claimed_path)
    }

    pub(crate) fn restore(&self) -> Result<()> {
        if self.claimed_path.exists() {
            self.queue.append_file_then_remove(&self.claimed_path)?;
            remove_file_if_exists(&self.failed_path)?;
        } else if self.failed_path.exists() {
            self.queue.append_file_then_remove(&self.failed_path)?;
        }
        Ok(())
    }
}

fn is_stale_claim(path: &Path, stem: &str, min_age: Duration) -> bool {
    if min_age.is_zero() {
        return true;
    }
    let Some((pid, epoch_nanos)) = claim_fields(path, stem) else {
        return false;
    };
    if process_alive(pid) {
        return false;
    }
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    now_nanos.saturating_sub(epoch_nanos) >= min_age.as_nanos()
}

fn claim_fields(path: &Path, stem: &str) -> Option<(i64, u128)> {
    let file_name = path.file_name()?.to_str()?;
    let rest = file_name.strip_prefix(&format!("{stem}.replay-"))?;
    let rest = rest.strip_suffix(".jsonl")?;
    let (before_sequence, _) = rest.rsplit_once('-')?;
    let (pid, nanos) = before_sequence.rsplit_once('-')?;
    Some((pid.parse().ok()?, nanos.parse().ok()?))
}

fn process_alive(pid: i64) -> bool {
    if pid <= 0 || pid > i64::from(i32::MAX) {
        return false;
    }
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn append_open_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
    }
}

#[cfg(test)]
#[path = "spill_queue/tests.rs"]
mod tests;
