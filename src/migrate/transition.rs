use anyhow::Result;
use rusqlite::Connection;

use super::state::{has_migration_table, mark_applied};
use super::types::OLD_BASELINE_VERSION;

pub(super) fn transition_from_old_system(conn: &Connection) -> Result<()> {
    if has_existing_migration_entries(conn) {
        return Ok(());
    }

    let old_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if old_version >= OLD_BASELINE_VERSION {
        crate::log::info(
            "migrate",
            &format!(
                "transitioning from user_version={} to _schema_migrations",
                old_version
            ),
        );
        mark_applied(conn, 1, "baseline")?;
    } else if old_version > 0 {
        anyhow::bail!(
            "Database is at schema v{}, but v{} is required. \
             Please upgrade to remem v0.3.7 first.",
            old_version,
            OLD_BASELINE_VERSION
        );
    }

    Ok(())
}

fn has_existing_migration_entries(conn: &Connection) -> bool {
    if !has_migration_table(conn) {
        return false;
    }

    conn.query_row("SELECT COUNT(*) FROM _schema_migrations", [], |row| {
        row.get(0)
    })
    .unwrap_or(0_i64)
        > 0
}
