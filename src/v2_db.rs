//! Open and migrate the v2 database file (`~/.remem/v2.sqlite`). Independent
//! from src/db/core which manages the v1 file (`remem.db`); both live in the
//! same data directory so admin commands can locate them together.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::migrate::run_v2_migrations;

const V2_DB_FILENAME: &str = "v2.sqlite";

/// Default path. Reuses `crate::db::data_dir()` so v1 and v2 sit side-by-side
/// (`~/.remem/remem.db` and `~/.remem/v2.sqlite`).
pub fn default_v2_db_path() -> PathBuf {
    crate::db::data_dir().join(V2_DB_FILENAME)
}

/// Open (creating if missing) the v2 database at `path`, run the v2 baseline
/// migration, and apply standard pragmas. Idempotent on re-open.
pub fn open_v2_db_at(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create v2 db parent {}", parent.display()))?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("open v2 db at {}", path.display()))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;
    run_v2_migrations(&conn)?;
    Ok(conn)
}

/// Open the default v2 database (`~/.remem/v2.sqlite`).
pub fn open_v2_db() -> Result<Connection> {
    open_v2_db_at(&default_v2_db_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("remem-v2-test-{pid}-{nonce}.sqlite"))
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn open_creates_file_and_applies_v2_baseline() {
        let path = unique_temp_path();
        let conn = open_v2_db_at(&path).expect("open v2 db");
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
            let _c = open_v2_db_at(&path).expect("first open");
        }
        let c = open_v2_db_at(&path).expect("second open");
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
        let _c = open_v2_db_at(&path).expect("open should not fail when dir exists");
        cleanup(&path);
    }
}
