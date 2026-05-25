use anyhow::Result;
use rusqlite::{backup::Backup, Connection};
use std::time::Duration;

use super::run::run_post_migration_hook;
use super::state::{applied_versions, has_migration_table};
use super::transition::backfill_to_baseline;
use super::types::{DryRunResult, Migration, MIGRATIONS, OLD_BASELINE_VERSION};

pub(crate) fn dry_run_pending(real_conn: &Connection) -> Result<DryRunResult> {
    let raw_current_version: i64 = real_conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);
    let applied = infer_applied_versions(real_conn, raw_current_version)?;
    let current_version = logical_current_version(raw_current_version, &applied);

    let mut test_conn = Connection::open_in_memory()?;
    if let Err(error) = clone_database(real_conn, &mut test_conn) {
        return Ok(DryRunResult {
            current_version,
            pending_count: applied_pending_count(&applied),
            error: Some(format!("database clone: {}", error)),
        });
    }
    if raw_current_version >= OLD_BASELINE_VERSION || has_migration_table(real_conn) {
        if let Err(error) = backfill_to_baseline(&test_conn) {
            return Ok(DryRunResult {
                current_version,
                pending_count: applied_pending_count(&applied),
                error: Some(format!("baseline backfill: {}", error)),
            });
        }
    }

    let pending: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|migration| !applied.contains(&migration.version))
        .collect();

    if pending.is_empty() {
        return Ok(DryRunResult {
            current_version,
            pending_count: 0,
            error: None,
        });
    }

    for migration in &pending {
        if let Err(error) = test_conn.execute_batch(migration.sql) {
            return Ok(DryRunResult {
                current_version,
                pending_count: pending.len(),
                error: Some(format!(
                    "v{:03}_{}: {}",
                    migration.version, migration.name, error
                )),
            });
        }
        if let Err(error) = run_post_migration_hook(&test_conn, migration.version, migration.name) {
            return Ok(DryRunResult {
                current_version,
                pending_count: pending.len(),
                error: Some(format!(
                    "v{:03}_{} post-migration hook: {}",
                    migration.version, migration.name, error
                )),
            });
        }
    }

    if let Err(error) = backfill_to_baseline(&test_conn) {
        return Ok(DryRunResult {
            current_version,
            pending_count: pending.len(),
            error: Some(format!("baseline backfill: {}", error)),
        });
    }

    Ok(DryRunResult {
        current_version,
        pending_count: pending.len(),
        error: None,
    })
}

fn applied_pending_count(applied: &[i64]) -> usize {
    MIGRATIONS
        .iter()
        .filter(|migration| !applied.contains(&migration.version))
        .count()
}

fn infer_applied_versions(conn: &Connection, current_version: i64) -> Result<Vec<i64>> {
    if has_migration_table(conn) {
        return applied_versions(conn);
    }
    if current_version >= OLD_BASELINE_VERSION {
        return Ok(vec![1]);
    }
    Ok(Vec::new())
}

fn logical_current_version(raw_current_version: i64, applied: &[i64]) -> i64 {
    let Some(latest_applied) = applied.iter().max() else {
        return raw_current_version;
    };
    raw_current_version.max(OLD_BASELINE_VERSION - 1 + latest_applied)
}

fn clone_database(src: &Connection, dst: &mut Connection) -> Result<()> {
    let backup = Backup::new(src, dst)?;
    backup.run_to_completion(100, Duration::from_millis(1), None)?;
    Ok(())
}
