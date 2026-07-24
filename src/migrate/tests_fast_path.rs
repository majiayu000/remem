use anyhow::Result;
use rusqlite::Connection;

use super::run_migrations;
use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path};

#[test]
fn run_migrations_on_current_schema_takes_no_write_lock() -> Result<()> {
    // Migrate a file-backed database to the latest schema up front.
    let path = unique_temp_db_path("migrate-current-nolock");
    let conn_a = Connection::open(&path)?;
    conn_a.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn_a)?;

    // A second connection holds the database write lock.
    let conn_b = Connection::open(&path)?;
    conn_b.busy_timeout(std::time::Duration::from_secs(0))?;
    conn_b.execute_batch("BEGIN IMMEDIATE")?;

    // With the schema already current, run_migrations must stay on the read-only
    // fast path and succeed immediately. Before the fast path it took the write
    // lock via BEGIN IMMEDIATE and would fail with SQLITE_BUSY here.
    conn_a.busy_timeout(std::time::Duration::from_secs(0))?;
    run_migrations(&conn_a)?;

    conn_b.execute_batch("ROLLBACK")?;
    cleanup_temp_db_files(&path);
    Ok(())
}
