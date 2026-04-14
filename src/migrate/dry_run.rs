use anyhow::Result;
use rusqlite::Connection;

use super::state::{applied_versions, has_migration_table};
use super::types::{DryRunResult, Migration, MIGRATIONS, OLD_BASELINE_VERSION};

pub(crate) fn dry_run_pending(real_conn: &Connection) -> Result<DryRunResult> {
    let current_version: i64 = real_conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);
    let applied = infer_applied_versions(real_conn, current_version)?;
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

    let test_conn = Connection::open_in_memory()?;
    let clone_errors = clone_schema(real_conn, &test_conn)?;
    if !clone_errors.is_empty() {
        return Ok(DryRunResult {
            current_version,
            pending_count: pending.len(),
            error: Some(format!(
                "clone_schema failures: {}",
                clone_errors.join("; ")
            )),
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

    Ok(DryRunResult {
        current_version,
        pending_count: pending.len(),
        error: None,
    })
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

fn clone_schema(src: &Connection, dst: &Connection) -> Result<Vec<String>> {
    let mut stmt = src.prepare(
        "SELECT sql FROM sqlite_master
         WHERE sql IS NOT NULL AND type IN ('table', 'index', 'trigger')",
    )?;
    let sqls: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|row| row.ok())
        .collect();

    let mut clone_errors: Vec<String> = Vec::new();
    for sql in &sqls {
        if sql.contains("fts5") || sql.starts_with("CREATE TABLE IF NOT EXISTS '_") {
            continue;
        }
        let safe = sql.replace("CREATE TABLE ", "CREATE TABLE IF NOT EXISTS ");
        let safe = safe.replace("CREATE INDEX ", "CREATE INDEX IF NOT EXISTS ");
        if let Err(error) = dst.execute_batch(&safe) {
            crate::log::debug("migrate", &format!("clone_schema skip: {}", error));
            clone_errors.push(format!("{}: {}", &sql[..sql.len().min(60)], error));
        }
    }
    Ok(clone_errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that a table whose DDL fails to clone is surfaced in clone_errors
    /// rather than silently dropped (issue #18).
    ///
    /// We use `PRAGMA writable_schema` to inject a DDL row with invalid SQL that
    /// SQLite accepted at insertion time but that `execute_batch` will reject.
    #[test]
    fn clone_schema_surfaces_non_fts5_clone_error() -> Result<()> {
        let src = Connection::open_in_memory()?;
        src.execute_batch("PRAGMA writable_schema = ON;")?;
        src.execute_batch(
            "INSERT INTO sqlite_master (type, name, tbl_name, rootpage, sql)
             VALUES ('table', 'bad_table', 'bad_table', 0, 'THIS IS NOT VALID SQL');",
        )?;

        let dst = Connection::open_in_memory()?;
        let errors = clone_schema(&src, &dst)?;
        assert!(
            !errors.is_empty(),
            "clone error for bad DDL must be surfaced"
        );
        assert!(
            errors[0].contains("bad_table") || errors[0].contains("THIS IS NOT"),
            "error message should reference the failing DDL: {:?}",
            errors
        );
        Ok(())
    }
}
