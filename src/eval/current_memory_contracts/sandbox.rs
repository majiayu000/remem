use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

pub(super) fn run_in_eval_sandbox<T>(run: impl FnOnce() -> Result<T>) -> Result<T> {
    let _env_guard = crate::runtime_config::ENV_LOCK
        .lock()
        .map_err(|error| anyhow::anyhow!("lock current-memory-contract eval env: {error}"))?;
    let data_dir = unique_temp_data_dir();
    std::fs::create_dir_all(&data_dir).with_context(|| {
        format!(
            "create current-memory-contract eval data dir {}",
            data_dir.display()
        )
    })?;
    let _config_restore = EnvRestore::remove("REMEM_CONFIG");
    let _embedding_restore = EnvRestore::set("REMEM_EMBEDDINGS_PROVIDER", "local");

    let result =
        crate::db::core::with_data_dir(&data_dir, || crate::log::with_log_dir(&data_dir, run));
    cleanup_data_dir_after_eval(&data_dir, result)
}

struct EnvRestore {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvRestore {
    fn set(key: &'static str, value: impl Into<OsString>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value.into());
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn unique_temp_data_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "remem-current-memory-contract-eval-{}-{nanos}",
        std::process::id()
    ))
}

fn cleanup_data_dir_after_eval<T>(data_dir: &Path, result: Result<T>) -> Result<T> {
    let cleanup = std::fs::remove_dir_all(data_dir).with_context(|| {
        format!(
            "remove current-memory-contract eval data dir {}",
            data_dir.display()
        )
    });
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(err)) => Err(err),
        (Err(err), Ok(())) => Err(err),
        (Err(err), Err(cleanup_err)) => {
            crate::log::warn(
                "eval-current-memory-contracts",
                &format!("cleanup failed after eval error: {cleanup_err}"),
            );
            Err(err)
        }
    }
}
