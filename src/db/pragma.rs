//! Connection pragmas shared by every way remem opens the store (GH949).
//!
//! Every configured connection parses the same environment policy before it
//! opens a database and then applies one typed pragma set. This keeps
//! read-write and read-only paths aligned without silently creating a database
//! when an operator supplied an invalid override.
//!
//! `PRAGMA optimize` is deliberately not wired here. It pays off on a
//! long-lived connection at close time, but the worker opens and drops a fresh
//! connection every loop iteration and hook subcommands are one-shot. The
//! daemon's periodic maintenance path is the appropriate future owner.

use std::env::VarError;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use rusqlite::Connection;

const CACHE_KIB_ENV: &str = "REMEM_SQLITE_CACHE_KIB";
const SYNCHRONOUS_ENV: &str = "REMEM_SQLITE_SYNCHRONOUS";
const DEFAULT_CACHE_KIB: i64 = 65_536;
const MAX_CACHE_KIB: i64 = 1_048_576;
const BUSY_TIMEOUT: Duration = Duration::from_millis(5_000);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionMode {
    ReadWrite,
    ReadOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Synchronous {
    Full,
    Normal,
}

impl Synchronous {
    fn pragma_value(self) -> &'static str {
        match self {
            Self::Full => "FULL",
            Self::Normal => "NORMAL",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionPragmas {
    cache_kib: i64,
    synchronous: Synchronous,
}

impl ConnectionPragmas {
    pub(crate) fn from_env() -> Result<Self> {
        let cache_kib = read_optional_env(CACHE_KIB_ENV)?;
        let synchronous = read_optional_env(SYNCHRONOUS_ENV)?;
        Self::from_values(cache_kib.as_deref(), synchronous.as_deref())
    }

    fn from_values(cache_kib: Option<&str>, synchronous: Option<&str>) -> Result<Self> {
        let cache_kib = match cache_kib.map(str::trim).filter(|value| !value.is_empty()) {
            None => DEFAULT_CACHE_KIB,
            Some(value) => {
                let parsed = value.parse::<i64>().with_context(|| {
                    format!("{CACHE_KIB_ENV} must be an integer from 1 through {MAX_CACHE_KIB} KiB")
                })?;
                ensure!(
                    (1..=MAX_CACHE_KIB).contains(&parsed),
                    "{CACHE_KIB_ENV} must be between 1 and {MAX_CACHE_KIB} KiB, got {parsed}"
                );
                parsed
            }
        };

        let synchronous = match synchronous.map(str::trim).filter(|value| !value.is_empty()) {
            None => Synchronous::Full,
            Some(value) if value.eq_ignore_ascii_case("full") => Synchronous::Full,
            Some(value) if value.eq_ignore_ascii_case("normal") => Synchronous::Normal,
            Some(value) => {
                bail!("{SYNCHRONOUS_ENV} must be `full` or `normal`, got `{value}`")
            }
        };

        Ok(Self {
            cache_kib,
            synchronous,
        })
    }

    pub(crate) fn apply(self, conn: &Connection, mode: ConnectionMode) -> Result<()> {
        if mode == ConnectionMode::ReadWrite {
            let journal_mode: String = conn
                .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
                .context("set SQLite journal_mode=WAL")?;
            ensure!(
                journal_mode.eq_ignore_ascii_case("wal"),
                "SQLite refused journal_mode=WAL and returned `{journal_mode}`"
            );
        }

        conn.pragma_update(None, "foreign_keys", "ON")
            .context("set SQLite foreign_keys=ON")?;
        conn.busy_timeout(BUSY_TIMEOUT)
            .context("set SQLite busy_timeout=5000")?;
        conn.pragma_update(None, "cache_size", -self.cache_kib)
            .with_context(|| format!("set SQLite cache_size=-{}", self.cache_kib))?;

        if mode == ConnectionMode::ReadWrite {
            conn.pragma_update(None, "synchronous", self.synchronous.pragma_value())
                .with_context(|| {
                    format!("set SQLite synchronous={}", self.synchronous.pragma_value())
                })?;
        }

        // SQLCipher does not guarantee that file-backed temp storage is
        // encrypted, so this is a security property rather than a tuning knob.
        conn.pragma_update(None, "temp_store", "MEMORY")
            .context("set SQLite temp_store=MEMORY")?;
        Ok(())
    }
}

fn read_optional_env(name: &'static str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => bail!("{name} must contain valid UTF-8"),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use rusqlite::{Connection, OpenFlags};

    use super::*;
    use crate::db::{
        core::{
            open_configured_connection, open_configured_existing_read_write_connection,
            open_configured_read_only_connection,
        },
        test_support::{cleanup_temp_db_files, unique_temp_db_path},
    };

    #[test]
    fn defaults_preserve_full_durability() {
        let pragmas = ConnectionPragmas::from_values(None, None).expect("default pragmas");
        assert_eq!(pragmas.cache_kib, DEFAULT_CACHE_KIB);
        assert_eq!(pragmas.synchronous, Synchronous::Full);
    }

    #[test]
    fn overrides_are_trimmed_and_case_insensitive() {
        let pragmas = ConnectionPragmas::from_values(Some(" 32768 "), Some(" Normal "))
            .expect("valid overrides");
        assert_eq!(pragmas.cache_kib, 32_768);
        assert_eq!(pragmas.synchronous, Synchronous::Normal);
    }

    #[test]
    fn cache_override_rejects_invalid_or_unsafe_values() {
        for value in ["words", "0", "-1", "1048577"] {
            let error = ConnectionPragmas::from_values(Some(value), None)
                .expect_err("invalid cache override must fail");
            assert!(
                error.to_string().contains(CACHE_KIB_ENV),
                "unexpected error for {value}: {error:#}"
            );
        }
    }

    #[test]
    fn synchronous_override_is_a_closed_enum() {
        for value in ["off", "extra", "1", "unexpected"] {
            let error = ConnectionPragmas::from_values(None, Some(value))
                .expect_err("invalid synchronous override must fail");
            assert!(
                error.to_string().contains(SYNCHRONOUS_ENV),
                "unexpected error for {value}: {error:#}"
            );
        }
    }

    #[test]
    fn read_write_open_paths_apply_default_pragmas() {
        let _lock = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("environment lock");
        let _env = PragmaEnvGuard::clean();
        let path = unique_temp_db_path("sqlite-pragmas-read-write");

        let create = open_configured_connection(&path, None).expect("create configured database");
        assert_default_read_write_pragmas(&create);
        drop(create);

        let existing = open_configured_existing_read_write_connection(&path, None)
            .expect("open existing configured database");
        assert_default_read_write_pragmas(&existing);
        drop(existing);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn read_only_open_path_applies_connection_local_pragmas() {
        let _lock = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("environment lock");
        let _env = PragmaEnvGuard::clean();
        let path = unique_temp_db_path("sqlite-pragmas-read-only");

        let create = open_configured_connection(&path, None).expect("create configured database");
        create
            .execute_batch("CREATE TABLE marker (value INTEGER); INSERT INTO marker VALUES (1);")
            .expect("seed database");
        drop(create);

        let read_only =
            open_configured_read_only_connection(&path, None).expect("open database read-only");
        assert_eq!(pragma_i64(&read_only, "foreign_keys"), 1);
        assert_eq!(pragma_i64(&read_only, "busy_timeout"), 5_000);
        assert_eq!(pragma_i64(&read_only, "cache_size"), -DEFAULT_CACHE_KIB);
        assert_eq!(pragma_i64(&read_only, "temp_store"), 2);
        assert_eq!(
            read_only
                .query_row("SELECT value FROM marker", [], |row| row.get::<_, i64>(0))
                .expect("read marker"),
            1
        );
        drop(read_only);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn environment_overrides_reach_runtime_pragmas() {
        let _lock = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("environment lock");
        let mut env = PragmaEnvGuard::clean();
        env.set(CACHE_KIB_ENV, "32768");
        env.set(SYNCHRONOUS_ENV, "normal");
        let path = unique_temp_db_path("sqlite-pragmas-env-overrides");
        let conn = open_configured_connection(&path, None).expect("open configured database");

        assert_eq!(pragma_i64(&conn, "cache_size"), -32_768);
        assert_eq!(pragma_i64(&conn, "synchronous"), 1);
        drop(conn);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn invalid_environment_fails_before_creating_database() {
        let _lock = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("environment lock");
        let mut env = PragmaEnvGuard::clean();
        env.set(CACHE_KIB_ENV, "not-a-number");
        let path = unique_temp_db_path("sqlite-pragmas-invalid-env");

        let error = open_configured_connection(&path, None)
            .expect_err("invalid environment must fail connection open");
        assert!(format!("{error:#}").contains(CACHE_KIB_ENV), "{error:#}");
        assert!(!path.exists(), "invalid config must not create a database");
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_environment_fails_before_creating_database() {
        use std::os::unix::ffi::OsStringExt;

        let _lock = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("environment lock");
        let _env = PragmaEnvGuard::clean();
        unsafe {
            std::env::set_var(CACHE_KIB_ENV, OsString::from_vec(vec![0xff]));
        }
        let path = unique_temp_db_path("sqlite-pragmas-non-unicode");

        let error = open_configured_connection(&path, None)
            .expect_err("non-Unicode environment must fail connection open");
        assert!(format!("{error:#}").contains("valid UTF-8"), "{error:#}");
        assert!(!path.exists(), "invalid config must not create a database");
    }

    #[test]
    fn read_only_tuning_does_not_require_database_writes() {
        let path = unique_temp_db_path("sqlite-pragmas-direct-read-only");
        Connection::open(&path)
            .expect("create database")
            .execute_batch("CREATE TABLE marker (value INTEGER);")
            .expect("create schema");
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open read-only");

        ConnectionPragmas::from_values(None, None)
            .expect("default pragmas")
            .apply(&conn, ConnectionMode::ReadOnly)
            .expect("apply read-only pragmas");
        assert_eq!(pragma_i64(&conn, "cache_size"), -DEFAULT_CACHE_KIB);
        assert_eq!(pragma_i64(&conn, "temp_store"), 2);
        drop(conn);
        cleanup_temp_db_files(&path);
    }

    fn assert_default_read_write_pragmas(conn: &Connection) {
        let journal_mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .expect("read journal_mode");
        assert_eq!(journal_mode, "wal");
        assert_eq!(pragma_i64(conn, "foreign_keys"), 1);
        assert_eq!(pragma_i64(conn, "busy_timeout"), 5_000);
        assert_eq!(pragma_i64(conn, "cache_size"), -DEFAULT_CACHE_KIB);
        assert_eq!(pragma_i64(conn, "synchronous"), 2);
        assert_eq!(pragma_i64(conn, "temp_store"), 2);
    }

    fn pragma_i64(conn: &Connection, name: &str) -> i64 {
        conn.pragma_query_value(None, name, |row| row.get(0))
            .unwrap_or_else(|error| panic!("read PRAGMA {name}: {error}"))
    }

    struct PragmaEnvGuard {
        previous: [(String, Option<OsString>); 2],
    }

    impl PragmaEnvGuard {
        fn clean() -> Self {
            let previous = [
                (CACHE_KIB_ENV.to_string(), std::env::var_os(CACHE_KIB_ENV)),
                (
                    SYNCHRONOUS_ENV.to_string(),
                    std::env::var_os(SYNCHRONOUS_ENV),
                ),
            ];
            unsafe {
                std::env::remove_var(CACHE_KIB_ENV);
                std::env::remove_var(SYNCHRONOUS_ENV);
            }
            Self { previous }
        }

        fn set(&mut self, name: &str, value: &str) {
            unsafe {
                std::env::set_var(name, value);
            }
        }
    }

    impl Drop for PragmaEnvGuard {
        fn drop(&mut self) {
            for (name, value) in &self.previous {
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }
}
