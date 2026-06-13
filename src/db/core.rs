use std::cell::RefCell;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

thread_local! {
    static DATA_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[cfg(test)]
static CONFIGURED_CONNECTION_OPENS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn reset_configured_connection_open_count() {
    CONFIGURED_CONNECTION_OPENS.store(0, Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) fn configured_connection_open_count() -> usize {
    CONFIGURED_CONNECTION_OPENS.load(Ordering::SeqCst)
}

#[cfg(test)]
fn record_configured_connection_open() {
    CONFIGURED_CONNECTION_OPENS.fetch_add(1, Ordering::SeqCst);
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

pub fn absolute_data_dir() -> Result<PathBuf> {
    let path = data_dir();
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(std::env::current_dir()
        .context("read current directory for relative REMEM_DATA_DIR")?
        .join(path))
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

    let conn = open_configured_connection(&path, key.as_ref())?;
    crate::retrieval::vector::load_vec_extension(&conn)?;
    crate::migrate::run_migrations(&conn)?;
    crate::retrieval::vector::ensure_vec_table(&conn)?;
    Ok(conn)
}

pub fn open_db_no_migrate() -> Result<Connection> {
    let path = db_path();
    let key = super::crypto::require_cipher_key_or_plaintext_override()?;
    if !path.exists() {
        anyhow::bail!("database not found: {}", path.display());
    }

    let conn = open_configured_existing_read_write_connection(&path, key.as_ref())?;
    crate::retrieval::vector::load_vec_extension(&conn)?;
    crate::migrate::ensure_schema_current(&conn)?;
    Ok(conn)
}

pub fn open_db_for_hook() -> Result<Connection> {
    let conn = open_db_no_migrate().context(
        "hook database open requires an existing current schema; run `remem install` or `remem migrate` outside the hook path",
    )?;
    let invariant_errors = crate::migrate::validate_schema_invariants(&conn)?;
    if !invariant_errors.is_empty() {
        anyhow::bail!(
            "hook database schema drift requires foreground migration: {}",
            invariant_errors.join("; ")
        );
    }
    Ok(conn)
}

pub fn open_db_read_only() -> Result<Connection> {
    let path = db_path();
    let key = super::crypto::require_cipher_key_or_plaintext_override()?;
    if !path.exists() {
        anyhow::bail!("database not found: {}", path.display());
    }

    open_configured_read_only_connection(&path, key.as_ref())
}

pub(crate) fn open_configured_connection(
    path: &Path,
    key: Option<&super::crypto::CipherKey>,
) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("Failed to open database: {}", path.display()))?;
    #[cfg(test)]
    record_configured_connection_open();

    super::crypto::configure_cipher(&conn, key)?;

    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;
    Ok(conn)
}

pub(crate) fn open_configured_existing_read_write_connection(
    path: &Path,
    key: Option<&super::crypto::CipherKey>,
) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).with_context(
        || {
            format!(
                "Failed to open existing database read-write: {}",
                path.display()
            )
        },
    )?;
    #[cfg(test)]
    record_configured_connection_open();

    super::crypto::configure_cipher(&conn, key)?;

    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;
    Ok(conn)
}

pub(crate) fn open_configured_read_only_connection(
    path: &Path,
    key: Option<&super::crypto::CipherKey>,
) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("Failed to open database read-only: {}", path.display()))?;
    #[cfg(test)]
    record_configured_connection_open();

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
    use rusqlite::{params, Connection};

    use super::*;
    use crate::db::crypto::ALLOW_PLAINTEXT_ENV;
    use crate::db::test_support::ScopedTestDataDir;

    #[test]
    fn open_db_for_hook_does_not_create_missing_database() {
        let test_dir = ScopedTestDataDir::new("hook-open-missing");
        test_dir.remove_db_files();

        let err = open_db_for_hook().expect_err("missing database should fail");

        let message = err.to_string();
        assert!(
            message.contains("hook database open requires"),
            "unexpected error: {message}"
        );
        assert!(
            !test_dir.path.exists(),
            "hook open must not create data dir"
        );
        assert!(
            !test_dir.db_path().exists(),
            "hook open must not create database file"
        );
    }

    #[test]
    fn open_db_for_hook_opens_current_schema_read_write() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("hook-open-current-rw");
        let setup = crate::db::open_db()?;
        drop(setup);

        let conn = crate::db::open_db_for_hook()?;
        conn.execute("CREATE TABLE hook_rw_probe(id INTEGER PRIMARY KEY)", [])?;
        conn.execute("INSERT INTO hook_rw_probe(id) VALUES (1)", [])?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM hook_rw_probe", [], |row| row.get(0))?;

        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn open_db_for_hook_rejects_older_schema_without_migrating() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("hook-open-older-schema");
        let setup = crate::db::open_db()?;
        let latest = crate::migrate::latest_schema_version();
        setup.execute(
            "DELETE FROM _schema_migrations WHERE version = ?1",
            [latest],
        )?;
        drop(setup);

        let err = crate::db::open_db_for_hook()
            .expect_err("older schema should require foreground migration");

        assert!(
            err.to_string().contains("hook database open requires"),
            "unexpected error: {err:#}"
        );
        let check = Connection::open(crate::db::db_path())?;
        let latest_rows: i64 = check.query_row(
            "SELECT COUNT(*) FROM _schema_migrations WHERE version = ?1",
            [latest],
            |row| row.get(0),
        )?;
        assert_eq!(latest_rows, 0);
        Ok(())
    }

    #[test]
    fn open_db_for_hook_rejects_schema_drift_without_repairing() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("hook-open-schema-drift");
        create_current_schema_with_v022_missing_objects(&test_dir.db_path())?;

        let err = crate::db::open_db_for_hook().expect_err("schema drift should fail closed");

        assert!(
            err.to_string().contains("hook database schema drift"),
            "unexpected error: {err:#}"
        );
        assert!(
            format!("{err:#}").contains("schema drift"),
            "unexpected error: {err:#}"
        );
        let check = Connection::open(test_dir.db_path())?;
        let state_keys_exists: i64 = check.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memory_state_keys'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(state_keys_exists, 0);
        Ok(())
    }

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
    fn open_db_read_only_refuses_plaintext_without_explicit_override() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("readonly-cipher-fail-closed");
        let conn = crate::db::open_db()?;
        drop(conn);
        std::env::remove_var(ALLOW_PLAINTEXT_ENV);

        let err = crate::db::open_db_read_only()
            .expect_err("read-only open must enforce the plaintext guard");

        let message = err.to_string();
        assert!(message.contains("SQLCipher key"), "got: {message}");
        assert!(
            test_dir.db_path().exists(),
            "read-only guard must not remove the existing database"
        );
        Ok(())
    }

    #[test]
    fn open_db_does_not_backfill_missing_vector_embeddings() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("open-no-vector-backfill");
        let conn = crate::db::open_db()?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (1, '/repo', 'Vector row', 'Open must not backfill this row.', 'decision', 1, 1, 'active')",
            [],
        )?;
        drop(conn);

        let reopened = crate::db::open_db()?;
        let count: i64 =
            reopened.query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
                row.get(0)
            })?;
        assert_eq!(count, 0);
        assert_eq!(
            crate::retrieval::vector::backfill_missing_memory_embeddings(&reopened, 10)?,
            1
        );
        Ok(())
    }

    #[test]
    fn open_db_read_only_does_not_run_migrations() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("readonly-no-migration");
        let path = crate::db::db_path();
        std::fs::create_dir_all(crate::db::data_dir())?;
        let conn = Connection::open(&path)?;
        conn.execute("CREATE TABLE marker (id INTEGER PRIMARY KEY)", [])?;
        drop(conn);

        let readonly = crate::db::open_db_read_only()?;
        let marker_exists: i64 = readonly.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'marker'",
            [],
            |row| row.get(0),
        )?;
        let migrations_exists: i64 = readonly.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )?;

        assert_eq!(marker_exists, 1);
        assert_eq!(migrations_exists, 0);
        Ok(())
    }

    #[test]
    fn open_db_no_migrate_does_not_create_missing_database() {
        let test_dir = ScopedTestDataDir::new("no-migrate-missing");
        test_dir.remove_db_files();

        let err = open_db_no_migrate().expect_err("missing database should fail");

        let message = err.to_string();
        assert!(
            message.contains("database not found"),
            "unexpected error: {message}"
        );
        assert!(
            !test_dir.path.exists(),
            "no-migrate open must not create data dir"
        );
        assert!(
            !test_dir.db_path().exists(),
            "no-migrate open must not create database file"
        );
    }

    #[test]
    fn open_db_no_migrate_refuses_plaintext_without_explicit_override() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("no-migrate-cipher-fail-closed");
        let conn = crate::db::open_db()?;
        drop(conn);
        std::env::remove_var(ALLOW_PLAINTEXT_ENV);

        let err = crate::db::open_db_no_migrate()
            .expect_err("no-migrate open must enforce the plaintext guard");

        let message = err.to_string();
        assert!(message.contains("SQLCipher key"), "got: {message}");
        assert!(
            test_dir.db_path().exists(),
            "no-migrate guard must not remove the existing database"
        );
        Ok(())
    }

    #[test]
    fn open_db_no_migrate_opens_current_schema_read_write() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("no-migrate-current-rw");
        let setup = crate::db::open_db()?;
        drop(setup);

        let conn = crate::db::open_db_no_migrate()?;
        conn.execute(
            "CREATE TABLE no_migrate_rw_probe(id INTEGER PRIMARY KEY)",
            [],
        )?;
        conn.execute("INSERT INTO no_migrate_rw_probe(id) VALUES (1)", [])?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM no_migrate_rw_probe", [], |row| {
            row.get(0)
        })?;

        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn open_db_no_migrate_does_not_run_migrations() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("no-migrate-no-migration");
        std::fs::create_dir_all(&test_dir.path)?;
        let setup = Connection::open(test_dir.db_path())?;
        setup.execute("CREATE TABLE marker (id INTEGER PRIMARY KEY)", [])?;
        drop(setup);

        let err = crate::db::open_db_no_migrate().expect_err("stale schema should fail");

        assert!(
            err.to_string().contains("schema is not initialized"),
            "unexpected error: {err:#}"
        );
        let check = Connection::open(test_dir.db_path())?;
        let migrations_exists: i64 = check.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_schema_migrations'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(migrations_exists, 0);
        Ok(())
    }

    #[test]
    fn open_db_no_migrate_rejects_older_schema_without_migrating() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("no-migrate-older-schema");
        let setup = crate::db::open_db()?;
        let latest = crate::migrate::latest_schema_version();
        setup.execute(
            "DELETE FROM _schema_migrations WHERE version = ?1",
            [latest],
        )?;
        drop(setup);

        let err = crate::db::open_db_no_migrate()
            .expect_err("older schema should require foreground migration");

        assert!(
            err.to_string().contains("requires schema"),
            "unexpected error: {err:#}"
        );
        let check = Connection::open(crate::db::db_path())?;
        let latest_rows: i64 = check.query_row(
            "SELECT COUNT(*) FROM _schema_migrations WHERE version = ?1",
            [latest],
            |row| row.get(0),
        )?;
        assert_eq!(latest_rows, 0);
        Ok(())
    }

    #[test]
    fn open_db_no_migrate_rejects_incomplete_schema_without_migrating() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("no-migrate-incomplete-schema");
        let setup = crate::db::open_db()?;
        let latest = crate::migrate::latest_schema_version();
        let missing = latest - 1;
        setup.execute(
            "DELETE FROM _schema_migrations WHERE version = ?1",
            [missing],
        )?;
        drop(setup);

        let err = crate::db::open_db_no_migrate()
            .expect_err("incomplete schema should require foreground migration");

        assert!(
            err.to_string().contains("missing migration"),
            "unexpected error: {err:#}"
        );
        let check = Connection::open(crate::db::db_path())?;
        let (missing_rows, latest_rows): (i64, i64) = check.query_row(
            "SELECT
                SUM(CASE WHEN version = ?1 THEN 1 ELSE 0 END),
                SUM(CASE WHEN version = ?2 THEN 1 ELSE 0 END)
             FROM _schema_migrations",
            (missing, latest),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(missing_rows, 0);
        assert_eq!(latest_rows, 1);
        Ok(())
    }

    #[test]
    fn open_db_no_migrate_rejects_newer_schema_without_migrating() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("no-migrate-newer-schema");
        let setup = crate::db::open_db()?;
        let latest = crate::migrate::latest_schema_version();
        setup.execute(
            "INSERT INTO _schema_migrations (version, name, applied_at_epoch)
             VALUES (?1, 'future-test', 0)",
            [latest + 1],
        )?;
        drop(setup);

        let err = crate::db::open_db_no_migrate().expect_err("newer schema should fail closed");

        assert!(
            err.to_string().contains("only knows up to"),
            "unexpected error: {err:#}"
        );
        Ok(())
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

    fn create_current_schema_with_v022_missing_objects(path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=OFF;")?;
        for migration in crate::migrate::MIGRATIONS
            .iter()
            .filter(|migration| migration.version != 22)
        {
            conn.execute_batch(migration.sql)?;
        }
        conn.execute_batch(
            "CREATE TABLE _schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at_epoch INTEGER NOT NULL
            );",
        )?;
        for migration in crate::migrate::MIGRATIONS {
            conn.execute(
                "INSERT INTO _schema_migrations (version, name, applied_at_epoch)
                 VALUES (?1, ?2, 1700000000)",
                params![migration.version, migration.name],
            )?;
        }
        conn.execute_batch(&format!(
            "PRAGMA user_version = {}; PRAGMA foreign_keys=ON;",
            crate::migrate::latest_schema_version()
        ))?;
        Ok(())
    }
}
