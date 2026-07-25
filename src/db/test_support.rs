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
    _config_guard: crate::runtime_config::TestEnvGuard,
    previous: Option<OsString>,
    previous_allow_plaintext: Option<OsString>,
    previous_cipher_key: Option<OsString>,
    pub path: PathBuf,
}

impl ScopedTestDataDir {
    pub fn new(label: &str) -> Self {
        let config_guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("runtime config test lock should acquire");
        let guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("REMEM_DATA_DIR");
        let previous_allow_plaintext = std::env::var_os("REMEM_ALLOW_PLAINTEXT_DB");
        let previous_cipher_key = std::env::var_os("REMEM_CIPHER_KEY");
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
            _config_guard: config_guard,
            previous,
            previous_allow_plaintext,
            previous_cipher_key,
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
        if let Some(previous) = self.previous_cipher_key.as_ref() {
            std::env::set_var("REMEM_CIPHER_KEY", previous);
        } else {
            std::env::remove_var("REMEM_CIPHER_KEY");
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

pub fn insert_legacy_pending_fixture(
    conn: &rusqlite::Connection,
    host: &str,
    session_id: &str,
    project: &str,
    tool_name: &str,
    tool_input: Option<&str>,
    tool_response: Option<&str>,
    cwd: Option<&str>,
) -> anyhow::Result<i64> {
    let epoch = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO pending_observations
         (host, session_id, project, tool_name, tool_input, tool_response, cwd,
          created_at_epoch, updated_at_epoch, status, attempt_count,
          next_retry_epoch, last_error, lease_owner, lease_expires_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 'pending', 0, NULL, NULL, NULL, NULL)",
        rusqlite::params![
            host,
            session_id,
            project,
            tool_name,
            tool_input,
            tool_response,
            cwd,
            epoch
        ],
    )?;
    Ok(conn.last_insert_rowid())
}
