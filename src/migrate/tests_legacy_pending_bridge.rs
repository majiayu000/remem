use anyhow::Result;
use rusqlite::Connection;

use super::run_migrations;

#[test]
fn v084_marks_empty_store_exhausted() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    assert_eq!(super::latest_schema_version(), 84);
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
