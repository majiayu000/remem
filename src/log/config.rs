use std::cell::RefCell;
use std::path::{Path, PathBuf};

pub(crate) const DEFAULT_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const LOG_ROTATION_KEEP: usize = 3;

thread_local! {
    static LOG_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub(crate) fn with_log_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = LogDirOverrideGuard::set(dir.to_path_buf());
    f()
}

pub(crate) fn log_path() -> Option<PathBuf> {
    if let Some(path) = LOG_DIR_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return Some(path.join("remem.log"));
    }
    Some(crate::db::data_dir().join("remem.log"))
}

pub(crate) fn log_max_bytes() -> u64 {
    std::env::var("REMEM_LOG_MAX_BYTES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_LOG_MAX_BYTES)
}

pub(crate) fn rotated_log_path(base: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}.{}", base.display(), index))
}

struct LogDirOverrideGuard {
    previous: Option<PathBuf>,
}

impl LogDirOverrideGuard {
    fn set(path: PathBuf) -> Self {
        let previous = LOG_DIR_OVERRIDE.with(|slot| slot.replace(Some(path)));
        Self { previous }
    }
}

impl Drop for LogDirOverrideGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        LOG_DIR_OVERRIDE.with(|slot| {
            slot.replace(previous);
        });
    }
}
