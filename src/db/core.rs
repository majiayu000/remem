use std::cell::RefCell;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

thread_local! {
    static DATA_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub(crate) fn with_data_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = DataDirOverrideGuard::set(dir.to_path_buf());
    f()
}

pub fn deterministic_hash(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;
    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

pub fn to_sql_refs(params: &[Box<dyn rusqlite::types::ToSql>]) -> Vec<&dyn rusqlite::types::ToSql> {
    params.iter().map(|b| b.as_ref()).collect()
}

pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn canonical_project_path(cwd: &str) -> PathBuf {
    crate::project_id::canonical_project_path(cwd)
}

pub fn project_from_cwd(cwd: &str) -> String {
    crate::project_id::project_from_cwd(cwd)
}

pub fn data_dir() -> PathBuf {
    if let Some(path) = DATA_DIR_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return path;
    }
    std::env::var("REMEM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".remem")
        })
}

pub fn db_path() -> PathBuf {
    data_dir().join("remem.db")
}

pub fn open_db() -> Result<Connection> {
    let path = db_path();
    let key = super::crypto::require_cipher_key_or_plaintext_override()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            if let Err(e) = std::fs::set_permissions(parent, perms) {
                crate::log::warn("db", &format!("cannot set data dir permissions: {}", e));
            }
        }
    }

    let conn = open_configured_connection(&path, key.as_deref())?;
    crate::retrieval::vector::load_vec_extension(&conn)?;
    crate::migrate::run_migrations(&conn)?;
    crate::retrieval::vector::ensure_vec_table(&conn)?;
    Ok(conn)
}

pub fn open_db_read_only() -> Result<Connection> {
    let path = db_path();
    let key = super::crypto::require_cipher_key_or_plaintext_override()?;
    if !path.exists() {
        anyhow::bail!("database not found: {}", path.display());
    }

    open_configured_read_only_connection(&path, key.as_deref())
}

pub(crate) fn open_configured_connection(path: &Path, key: Option<&str>) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("Failed to open database: {}", path.display()))?;

    super::crypto::configure_cipher(&conn, key)?;

    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;
    Ok(conn)
}

pub(crate) fn open_configured_read_only_connection(
    path: &Path,
    key: Option<&str>,
) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("Failed to open database read-only: {}", path.display()))?;

    super::crypto::configure_cipher(&conn, key)?;

    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    Ok(conn)
}

pub fn detect_git_branch(cwd: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch)
    }
}

struct DataDirOverrideGuard {
    previous: Option<PathBuf>,
}

impl DataDirOverrideGuard {
    fn set(path: PathBuf) -> Self {
        let previous = DATA_DIR_OVERRIDE.with(|slot| slot.replace(Some(path)));
        Self { previous }
    }
}

impl Drop for DataDirOverrideGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        DATA_DIR_OVERRIDE.with(|slot| {
            slot.replace(previous);
        });
    }
}

pub fn detect_git_commit(cwd: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::db::test_support::ScopedTestDataDir;

    #[test]
    fn open_db_read_only_does_not_create_missing_database() {
        let test_dir = ScopedTestDataDir::new("readonly-missing");
        test_dir.remove_db_files();

        let err = open_db_read_only().expect_err("missing database should fail");

        let message = err.to_string();
        assert!(
            message.contains("database not found"),
            "unexpected error: {message}"
        );
        assert!(
            !test_dir.path.exists(),
            "read-only open must not create data dir"
        );
        assert!(
            !test_dir.db_path().exists(),
            "read-only open must not create database file"
        );
    }

    #[test]
    fn open_db_read_only_opens_existing_database_without_write_access() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("readonly-existing");
        std::fs::create_dir_all(&test_dir.path)?;
        let setup = Connection::open(test_dir.db_path())?;
        setup.execute_batch(
            "CREATE TABLE readonly_probe(id INTEGER PRIMARY KEY);
             INSERT INTO readonly_probe(id) VALUES (1);",
        )?;
        drop(setup);

        let conn = open_db_read_only()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM readonly_probe", [], |row| row.get(0))?;
        assert_eq!(count, 1);

        let err = conn
            .execute("INSERT INTO readonly_probe(id) VALUES (2)", [])
            .expect_err("read-only connection must reject writes");
        assert_eq!(err.sqlite_error_code(), Some(rusqlite::ErrorCode::ReadOnly));
        Ok(())
    }
}
