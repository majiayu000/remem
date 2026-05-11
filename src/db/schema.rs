//! Open and migrate the schema database file (`~/.remem/schema.sqlite`).
//! This is the durable database boundary for the normalized memory schema.

pub mod gate;
pub mod import;
mod migrate;
pub mod status;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const SCHEMA_DB_FILENAME: &str = "schema.sqlite";

/// Default path for the normalized schema database.
pub fn default_path() -> PathBuf {
    crate::db::data_dir().join(SCHEMA_DB_FILENAME)
}

/// Open the schema database at `path`, creating and migrating it if needed.
/// Idempotent on re-open.
pub fn open_at(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create schema db parent {}", parent.display()))?;
    }
    let conn =
        Connection::open(path).with_context(|| format!("open schema db at {}", path.display()))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;
    migrate::run_migrations(&conn)?;
    Ok(conn)
}

/// Open the default schema database.
pub fn open() -> Result<Connection> {
    open_at(&default_path())
}

/// Drop the schema database file at `path` (along with WAL/SHM sidecars),
/// then re-create it via `open_at`. Caller is responsible for confirming the
/// destructive intent.
pub fn reset_at(path: &Path) -> Result<()> {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    let _ = open_at(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{cleanup_temp_db_files as cleanup, unique_temp_db_path};

    fn unique_temp_path() -> PathBuf {
        unique_temp_db_path("schema-db")
    }

    #[test]
    fn open_creates_file_and_applies_schema_baseline() {
        let path = unique_temp_path();
        let conn = open_at(&path).expect("open schema db");
        assert!(path.exists(), "db file should exist at {}", path.display());
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='hosts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        cleanup(&path);
    }

    #[test]
    fn reopen_is_idempotent() {
        let path = unique_temp_path();
        {
            let _c = open_at(&path).expect("first open");
        }
        let c = open_at(&path).expect("second open");
        let host_count: i64 = c
            .query_row("SELECT COUNT(*) FROM hosts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(host_count, 2, "seed must not duplicate on re-open");
        cleanup(&path);
    }

    #[test]
    fn open_succeeds_after_dir_already_exists() {
        let path = unique_temp_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let _c = open_at(&path).expect("open should not fail when dir exists");
        cleanup(&path);
    }

    #[test]
    fn reset_drops_existing_schema_data_and_reseeds() {
        let path = unique_temp_path();
        {
            let conn = open_at(&path).unwrap();
            conn.execute(
                "INSERT INTO hosts(name, enabled, created_at_epoch) VALUES ('extra', 1, 0)",
                [],
            )
            .unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM hosts", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 3, "seeded 2 + extra 1");
        }
        reset_at(&path).expect("reset");
        let conn = open_at(&path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM hosts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2, "after reset only the 2 seed rows remain");
        cleanup(&path);
    }
}
