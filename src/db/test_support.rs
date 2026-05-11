use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub struct ScopedTestDataDir {
    _guard: MutexGuard<'static, ()>,
    previous: Option<OsString>,
    pub path: PathBuf,
}

impl ScopedTestDataDir {
    pub fn new(label: &str) -> Self {
        let guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("REMEM_DATA_DIR");
        let unique = format!(
            "remem-test-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        let _ = std::fs::remove_dir_all(&path);
        std::env::set_var("REMEM_DATA_DIR", &path);
        Self {
            _guard: guard,
            previous,
            path,
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.path.join("remem.db")
    }

    pub fn remove_db_files(&self) {
        let db_path = self.db_path();
        let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
        let shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
        for path in [db_path, wal_path, shm_path] {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for ScopedTestDataDir {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var("REMEM_DATA_DIR", previous);
        } else {
            std::env::remove_var("REMEM_DATA_DIR");
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Generate a unique temp file path for a test sqlite db. Used by v2 db /
/// admin / v2 import test modules; nonce is `pid + nanos` so concurrent
/// `cargo test` runs do not collide. Caller owns cleanup (pair with
/// `cleanup_temp_db_files` for `-wal` / `-shm` sidecars).
pub fn unique_temp_db_path(label: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "remem-{label}-{}-{}.sqlite",
        std::process::id(),
        nonce
    ))
}

/// Remove a sqlite test file along with its `-wal` / `-shm` sidecars.
pub fn cleanup_temp_db_files(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}
