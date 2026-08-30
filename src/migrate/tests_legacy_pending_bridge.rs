use anyhow::Result;
use rusqlite::Connection;

use super::run_migrations;

fn run_through_v084(conn: &Connection) -> Result<()> {
    for migration in &super::MIGRATIONS[..84] {
        conn.execute_batch(migration.sql)?;
    }
    Ok(())
}

#[test]
fn v085_marks_empty_store_exhausted() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    assert_eq!(super::latest_schema_version(), 91);
    let (state, residual): (String, i64) = conn.query_row(
        "SELECT state, residual_count FROM legacy_surface_state
         WHERE surface = 'pending_observations'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(state, "exhausted");
    assert_eq!(residual, 0);
    Ok(())
}

#[test]
fn v085_keeps_delayed_and_leased_residual_rows_draining() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    run_through_v084(&conn)?;
    let now = chrono::Utc::now().timestamp();
    for (session, status) in [
        ("delayed-pending", "pending"),
        ("active-processing", "processing"),
        ("delayed-failed", "failed"),
    ] {
        conn.execute(
            "INSERT INTO pending_observations (
                 host, session_id, project, tool_name, created_at_epoch,
                 updated_at_epoch, status, next_retry_epoch, lease_owner,
                 lease_expires_epoch, failure_class
             ) VALUES ('codex-cli', ?1, 'alpha', 'Bash', ?2, ?2, ?3, ?4,
                       'live-worker', ?4, 'transient')",
            rusqlite::params![session, now, status, now + 300],
        )?;
    }

    conn.execute_batch(super::MIGRATIONS[84].sql)?;

    let (state, residual): (String, i64) = conn.query_row(
        "SELECT state, residual_count FROM legacy_surface_state
         WHERE surface = 'pending_observations'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(state, "frozen_draining");
    assert_eq!(residual, 3);
    Ok(())
}
