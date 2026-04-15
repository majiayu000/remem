use anyhow::Result;
use rusqlite::Connection;

use super::state::{applied_versions, has_migration_table};
use super::transition::backfill_to_baseline;
use super::types::{DryRunResult, Migration, MIGRATIONS, OLD_BASELINE_VERSION};

pub(crate) fn dry_run_pending(real_conn: &Connection) -> Result<DryRunResult> {
    let current_version: i64 = real_conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);
    let applied = infer_applied_versions(real_conn, current_version)?;

    let test_conn = Connection::open_in_memory()?;
    if let Err(error) = clone_schema(real_conn, &test_conn) {
        return Ok(DryRunResult {
            current_version,
            pending_count: applied_pending_count(&applied),
            error: Some(format!("schema clone: {}", error)),
        });
    }
    if current_version >= OLD_BASELINE_VERSION || has_migration_table(real_conn) {
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

pub(super) fn clone_schema(src: &Connection, dst: &Connection) -> Result<()> {
    let mut stmt = src.prepare(
        "SELECT name, sql FROM sqlite_master
         WHERE sql IS NOT NULL AND type IN ('table', 'index', 'trigger')
         ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 WHEN 'trigger' THEN 2 ELSE 3 END, name",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|row| row.ok())
        .collect();

    for (name, sql) in &rows {
        // Skip FTS5 virtual tables and internal tables whose names begin with '_'.
        // Check the object name from sqlite_master directly — scanning the SQL body
        // would produce false positives for string literals like DEFAULT '_pending'.
        if sql.contains("fts5") || name.starts_with('_') {
            continue;
        }
        let safe = sql.replace("CREATE TABLE ", "CREATE TABLE IF NOT EXISTS ");
        let safe = safe.replace("CREATE INDEX ", "CREATE INDEX IF NOT EXISTS ");
        dst.execute_batch(&safe)?;
    }
    Ok(())
}
