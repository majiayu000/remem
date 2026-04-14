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

    // Mirror what run_migrations() does via transition_from_old_system(): backfill
    // baseline columns/tables onto the clone so that pending migrations replay
    // against the same state the real upgrade path would see.  Without this, an
    // older live database that is missing baseline-added columns causes dry-run to
    // report false failures.
    //
    // Production (transition.rs) backfills for any old_version > 0, covering v1-v12
    // as well as v13+.  The previous condition only covered current_version >=
    // OLD_BASELINE_VERSION (v13), leaving v1-v12 databases diverged from production
    // and still producing false migration failures.
    if current_version > 0 || has_migration_table(real_conn) {
        if let Err(error) = backfill_to_baseline(&test_conn) {
            return Ok(DryRunResult {
                current_version,
                pending_count: pending.len(),
                error: Some(format!("baseline backfill: {}", error)),
            });
        }
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
            let preview: String = sql.chars().take(60).collect();
            clone_errors.push(format!("{}: {}", preview, error));
        }
    }
    Ok(clone_errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database with user_version in [1, OLD_BASELINE_VERSION) (v1-v12) must receive
    /// the same baseline backfill as production: `transition_from_old_system` runs
    /// `backfill_to_baseline` for any `old_version > 0`.  The previous dry-run condition
    /// only covered `>= OLD_BASELINE_VERSION`, so v1-v12 databases diverged from the
    /// production upgrade path and could keep reporting false migration failures.
    #[test]
    fn dry_run_backfills_for_legacy_v1_to_v12_database() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        // Build a minimal v5-era schema: core tables exist with their original column
        // set, missing the newer columns that backfill_to_baseline will add.
        // All columns referenced by the index creation batch in backfill_to_baseline
        // must either be present initially or be added first by add_column_if_missing.
        conn.execute_batch(
            "CREATE TABLE sdk_sessions (
                 id INTEGER PRIMARY KEY, memory_session_id TEXT NOT NULL,
                 project TEXT, started_at_epoch INTEGER, status TEXT DEFAULT 'active'
             );
             CREATE TABLE observations (
                 id INTEGER PRIMARY KEY, memory_session_id TEXT NOT NULL,
                 project TEXT NOT NULL, type TEXT NOT NULL, title TEXT,
                 created_at_epoch INTEGER
             );
             CREATE TABLE memories (
                 id INTEGER PRIMARY KEY, project TEXT NOT NULL,
                 memory_type TEXT, topic_key TEXT, status TEXT,
                 updated_at_epoch INTEGER
             );
             CREATE TABLE pending_observations (
                 id INTEGER PRIMARY KEY, session_id TEXT NOT NULL,
                 project TEXT, memory_session_id TEXT,
                 created_at_epoch INTEGER, lease_expires_epoch INTEGER
             );
             CREATE TABLE events (
                 id INTEGER PRIMARY KEY, session_id TEXT,
                 project TEXT, created_at_epoch INTEGER
             );
             CREATE TABLE session_summaries (
                 id INTEGER PRIMARY KEY, project TEXT, created_at_epoch INTEGER
             );
             PRAGMA user_version = 5;",
        )?;
        let result = dry_run_pending(&conn)?;
        assert!(
            result.error.is_none(),
            "dry_run on a v1-v12 legacy database must not return an error; got: {:?}",
            result.error
        );
        Ok(())
    }

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
