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

fn clone_schema(src: &Connection, dst: &Connection) -> Result<()> {
    let mut stmt = src.prepare(
        "SELECT sql, tbl_name FROM sqlite_master
         WHERE sql IS NOT NULL AND type IN ('table', 'index', 'trigger')
         ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 WHEN 'trigger' THEN 2 ELSE 3 END, name",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|row| row.ok())
        .collect();

    for (sql, tbl_name) in &rows {
        // Skip fts5 virtual tables and any object belonging to an _-prefixed internal table.
        // tbl_name is the owning table for indexes/triggers too, so this covers all three types.
        if sql.contains("fts5") || tbl_name.starts_with('_') {
            continue;
        }
        let safe = sql.replace("CREATE TABLE ", "CREATE TABLE IF NOT EXISTS ");
        let safe = safe.replace("CREATE INDEX ", "CREATE INDEX IF NOT EXISTS ");
        dst.execute_batch(&safe)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_schema_skips_underscore_prefixed_tables() {
        let src = Connection::open_in_memory().unwrap();
        // quoted underscore table
        src.execute_batch("CREATE TABLE '_internal' (id INTEGER PRIMARY KEY)")
            .unwrap();
        // unquoted underscore table (e.g. _schema_migrations stored by state.rs)
        src.execute_batch("CREATE TABLE _schema_migrations (version INTEGER PRIMARY KEY)")
            .unwrap();
        // index on an underscore table — must not be cloned either
        src.execute_batch("CREATE INDEX idx_sm_version ON _schema_migrations(version)")
            .unwrap();
        src.execute_batch("CREATE TABLE normal (id INTEGER PRIMARY KEY)")
            .unwrap();

        let dst = Connection::open_in_memory().unwrap();
        clone_schema(&src, &dst).unwrap();

        let count: i64 = dst
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = '_internal'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "_internal table must not be cloned");

        let count: i64 = dst
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = '_schema_migrations'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "_schema_migrations table must not be cloned");

        let count: i64 = dst
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'idx_sm_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "index on _schema_migrations must not be cloned");

        let count: i64 = dst
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'normal'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "normal table must be cloned");
    }
}
