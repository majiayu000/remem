use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;

use super::state::{applied_versions, ensure_migration_table, mark_applied};
use super::transition::transition_from_old_system;
use super::types::{MIGRATIONS, OLD_BASELINE_VERSION};

pub(crate) fn run_migrations(conn: &Connection) -> Result<()> {
    ensure_migration_table(conn)?;

    conn.execute_batch("BEGIN IMMEDIATE")
        .context("begin migration transaction")?;
    let result = run_migrations_locked(conn);
    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")
                .context("commit migration transaction")?;
            Ok(())
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute_batch("ROLLBACK") {
                crate::log::warn(
                    "migrate",
                    &format!("rollback failed after migration error: {rollback_error}"),
                );
            }
            Err(error)
        }
    }
}

fn run_migrations_locked(conn: &Connection) -> Result<()> {
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
        run_post_migration_hook(conn, migration.version, migration.name)?;
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
    let current_user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);
    let user_version = current_user_version.max(OLD_BASELINE_VERSION - 1 + latest);
    conn.execute_batch(&format!("PRAGMA user_version = {}", user_version))?;
    Ok(())
}

fn run_post_migration_hook(conn: &Connection, version: i64, name: &str) -> Result<()> {
    if version == 15 {
        let rebuilt = crate::memory::search_context::rebuild_all(conn).with_context(|| {
            format!("migration v{version:03}_{name} failed to rebuild memory search contexts")
        })?;
        crate::log::info(
            "migrate",
            &format!("rebuilt search_context for {rebuilt} memories"),
        );
    }
    Ok(())
}
