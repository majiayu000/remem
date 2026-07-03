use std::cell::RefCell;
use std::path::{Path, PathBuf};

pub(crate) const DEFAULT_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const DEFAULT_LOG_MAX_ROTATED_FILES: usize = 3;
pub(crate) const DEFAULT_LOG_LOCK_TIMEOUT_MS: u64 = 250;

thread_local! {
    static LOG_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct InvalidLogEnv {
    pub name: &'static str,
    pub default: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct LogPolicy {
    pub path: PathBuf,
    pub max_bytes: u64,
    pub max_rotated_files: usize,
    pub lock_timeout_ms: u64,
    pub invalid_env: Vec<InvalidLogEnv>,
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

pub(crate) fn log_policy() -> Option<LogPolicy> {
    let path = log_path()?;
    let mut invalid_env = Vec::new();
    let max_bytes = parse_positive_u64_env(
        "REMEM_LOG_MAX_BYTES",
        DEFAULT_LOG_MAX_BYTES,
        &mut invalid_env,
    );
    let max_rotated_files = parse_non_negative_usize_env(
        "REMEM_LOG_MAX_ROTATED_FILES",
        DEFAULT_LOG_MAX_ROTATED_FILES,
        &mut invalid_env,
    );
    let lock_timeout_ms = parse_positive_u64_env(
        "REMEM_LOG_LOCK_TIMEOUT_MS",
        DEFAULT_LOG_LOCK_TIMEOUT_MS,
        &mut invalid_env,
    );
    Some(LogPolicy {
        path,
        max_bytes,
        max_rotated_files,
        lock_timeout_ms,
        invalid_env,
    })
}

#[cfg(test)]
pub(crate) fn log_max_bytes() -> u64 {
    log_policy()
        .map(|policy| policy.max_bytes)
        .unwrap_or(DEFAULT_LOG_MAX_BYTES)
}

pub(crate) fn rotated_log_path(base: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}.{}", base.display(), index))
}

pub(crate) fn log_lock_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.lock", base.display()))
}

pub(crate) fn log_rotation_issue_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.rotation-issue.json", base.display()))
}

fn parse_positive_u64_env(
    name: &'static str,
    default: u64,
    invalid_env: &mut Vec<InvalidLogEnv>,
) -> u64 {
    let Ok(value) = std::env::var(name) else {
        return default;
    };
    match value.parse::<u64>() {
        Ok(parsed) if parsed > 0 => parsed,
        _ => {
            invalid_env.push(InvalidLogEnv {
                name,
                default: default.to_string(),
                reason: "expected positive integer",
            });
            default
        }
    }
}

fn parse_non_negative_usize_env(
    name: &'static str,
    default: usize,
    invalid_env: &mut Vec<InvalidLogEnv>,
) -> usize {
    let Ok(value) = std::env::var(name) else {
        return default;
    };
    match value.parse::<usize>() {
        Ok(parsed) => parsed,
        Err(_) => {
            invalid_env.push(InvalidLogEnv {
                name,
                default: default.to_string(),
                reason: "expected non-negative integer",
            });
            default
        }
    }
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
