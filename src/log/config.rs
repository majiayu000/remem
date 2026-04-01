use std::path::{Path, PathBuf};

pub(crate) const DEFAULT_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const LOG_ROTATION_KEEP: usize = 3;

pub(crate) fn log_path() -> Option<PathBuf> {
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
