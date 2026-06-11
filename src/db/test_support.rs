use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, MutexGuard, OnceLock,
};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub struct ScopedTestDataDir {
    _guard: MutexGuard<'static, ()>,
    previous: Option<OsString>,
    previous_allow_plaintext: Option<OsString>,
    pub path: PathBuf,
}

impl ScopedTestDataDir {
    pub fn new(label: &str) -> Self {
        let guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("REMEM_DATA_DIR");
        let previous_allow_plaintext = std::env::var_os("REMEM_ALLOW_PLAINTEXT_DB");
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
        std::env::set_var("REMEM_ALLOW_PLAINTEXT_DB", "1");
        Self {
            _guard: guard,
            previous,
            previous_allow_plaintext,
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
        if let Some(previous) = self.previous_allow_plaintext.as_ref() {
            std::env::set_var("REMEM_ALLOW_PLAINTEXT_DB", previous);
        } else {
            std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Generate a unique temp file path for a test sqlite db. Used by schema,
/// admin, and import test modules; nonce is `pid + nanos` so concurrent
/// `cargo test` runs do not collide. Caller owns cleanup (pair with
/// `cleanup_temp_db_files` for `-wal` / `-shm` sidecars).
pub fn unique_temp_db_path(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "remem-{label}-{}-{nonce}-{counter}.sqlite",
        std::process::id(),
    ))
}

/// Remove a sqlite test file along with its `-wal` / `-shm` sidecars.
pub fn cleanup_temp_db_files(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}

pub fn runtime_connection() -> anyhow::Result<rusqlite::Connection> {
    crate::db::open_db()
}

pub fn reset_runtime_connection_open_count() {
    crate::db::core::reset_configured_connection_open_count();
}

pub fn runtime_connection_open_count() -> usize {
    crate::db::core::configured_connection_open_count()
}
