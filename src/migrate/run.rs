use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;

use super::state::{applied_versions, ensure_migration_table, mark_applied};
use super::transition::transition_from_old_system;
use super::types::{MIGRATIONS, OLD_BASELINE_VERSION};

pub(crate) fn run_migrations(conn: &Connection) -> Result<()> {
    ensure_migration_table(conn)?;
    transition_from_old_system(conn)?;

    let applied = applied_versions(conn)?;
    let binary_latest = MIGRATIONS
        .last()
        .map(|migration| migration.version)
        .unwrap_or(0);
    let db_latest = applied.iter().copied().max().unwrap_or(0);
    if db_latest > binary_latest {
        return Err(anyhow!(
            "database is at schema v{db_latest} but this binary only knows up to v{binary_latest}; please upgrade remem"
        ));
    }
    for migration in MIGRATIONS {
        if applied.contains(&migration.version) {
            continue;
        }
        crate::log::info(
            "migrate",
            &format!("applying v{:03}_{}", migration.version, migration.name),
        );
        conn.execute_batch(migration.sql).with_context(|| {
            format!(
                "migration v{:03}_{} failed",
                migration.version, migration.name
            )
        })?;
        mark_applied(conn, migration.version, migration.name)?;
        crate::log::info(
            "migrate",
            &format!("applied v{:03}_{}", migration.version, migration.name),
        );
    }

    let latest = MIGRATIONS
        .last()
        .map(|migration| migration.version)
        .unwrap_or(0);
    let user_version = OLD_BASELINE_VERSION - 1 + latest;
    conn.execute_batch(&format!("PRAGMA user_version = {}", user_version))?;
    Ok(())
}
