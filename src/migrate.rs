//! Migration framework: numbered, immutable SQL files compiled into the binary.
//!
//! Each migration runs once and is tracked in `_schema_migrations`.
//! Transition from the old `PRAGMA user_version` system is automatic.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

/// A single schema migration.
pub(crate) struct Migration {
    /// Monotonically increasing version number (1, 2, 3, ...).
    pub version: i64,
    /// Human-readable name (used in tracking table and logs).
    pub name: &'static str,
    /// SQL to execute.
    pub sql: &'static str,
}

/// All migrations in order. Append-only — never modify published entries.
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "baseline",
    sql: include_str!("migrations/v001_baseline.sql"),
}];

/// The old user_version value that the baseline migration corresponds to.
/// Existing databases at or above this version skip the baseline.
const OLD_BASELINE_VERSION: i64 = 13;

/// Run all pending migrations. Called from `open_db()`.
pub(crate) fn run_migrations(conn: &Connection) -> Result<()> {
    ensure_migration_table(conn)?;
    transition_from_old_system(conn)?;

    let applied = applied_versions(conn)?;
    for m in MIGRATIONS {
        if applied.contains(&m.version) {
            continue;
        }
        crate::log::info("migrate", &format!("applying v{:03}_{}", m.version, m.name));
        conn.execute_batch(m.sql)
            .with_context(|| format!("migration v{:03}_{} failed", m.version, m.name))?;
        mark_applied(conn, m.version, m.name)?;
        crate::log::info("migrate", &format!("applied v{:03}_{}", m.version, m.name));
    }

    // Update user_version for backwards compatibility
    let latest = MIGRATIONS.last().map(|m| m.version).unwrap_or(0);
    let uv = OLD_BASELINE_VERSION - 1 + latest; // v1 → 13, v2 → 14, ...
    conn.execute_batch(&format!("PRAGMA user_version = {}", uv))?;

    Ok(())
}

/// Dry-run all pending migrations on an in-memory copy of the real schema.
/// Returns Ok(()) if all migrations would succeed, Err with details otherwise.
pub(crate) fn dry_run_pending(real_conn: &Connection) -> Result<DryRunResult> {
    let real_version: i64 = real_conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);

    // Determine which migrations would run
    let applied = if has_migration_table(real_conn) {
        applied_versions(real_conn)?
    } else if real_version >= OLD_BASELINE_VERSION {
        vec![1] // baseline would be skipped
    } else {
        vec![] // all migrations would run
    };

    let pending: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|m| !applied.contains(&m.version))
        .collect();

    if pending.is_empty() {
        return Ok(DryRunResult {
            current_version: real_version,
            pending_count: 0,
            error: None,
        });
    }

    // Copy real table schemas into an in-memory DB and run migrations
    let test_conn = Connection::open_in_memory()?;
    clone_schema(real_conn, &test_conn)?;

    for m in &pending {
        if let Err(e) = test_conn.execute_batch(m.sql) {
            return Ok(DryRunResult {
                current_version: real_version,
                pending_count: pending.len(),
                error: Some(format!("v{:03}_{}: {}", m.version, m.name, e)),
            });
        }
    }

    Ok(DryRunResult {
        current_version: real_version,
        pending_count: pending.len(),
        error: None,
    })
}

pub(crate) struct DryRunResult {
    pub current_version: i64,
    pub pending_count: usize,
    pub error: Option<String>,
}

// --- internals ---

fn ensure_migration_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at_epoch INTEGER NOT NULL
        )",
    )?;
    Ok(())
}

fn has_migration_table(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='_schema_migrations'",
        [],
        |_| Ok(()),
    )
    .is_ok()
}

fn applied_versions(conn: &Connection) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT version FROM _schema_migrations ORDER BY version")?;
    let versions: Vec<i64> = stmt
        .query_map([], |r| r.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(versions)
}

fn mark_applied(conn: &Connection, version: i64, name: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT OR IGNORE INTO _schema_migrations (version, name, applied_at_epoch) VALUES (?1, ?2, ?3)",
        params![version, name, now],
    )?;
    Ok(())
}

/// Transition from the old `PRAGMA user_version` system.
/// - user_version >= 13: mark baseline as applied (DB is current)
/// - user_version 1..12: reject — user must upgrade to v0.3.7 first
/// - user_version 0: new DB, baseline will run normally
fn transition_from_old_system(conn: &Connection) -> Result<()> {
    // Already transitioned?
    if has_migration_table(conn) {
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM _schema_migrations", [], |r| r.get(0))
            .unwrap_or(0);
        if count > 0 {
            return Ok(());
        }
    }

    let old_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    if old_version >= OLD_BASELINE_VERSION {
        // Existing DB with current schema — skip baseline
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
    // old_version == 0 → new DB, baseline will run

    Ok(())
}

/// Copy table/index/trigger schemas from real DB to test DB (for dry-run).
fn clone_schema(src: &Connection, dst: &Connection) -> Result<()> {
    let mut stmt = src.prepare(
        "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND type IN ('table', 'index', 'trigger')",
    )?;
    let sqls: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    for sql in &sqls {
        // Skip FTS shadow tables and internal tables
        if sql.contains("fts5") || sql.starts_with("CREATE TABLE IF NOT EXISTS '_") {
            continue;
        }
        // Make CREATE TABLE idempotent
        let safe = sql.replace("CREATE TABLE ", "CREATE TABLE IF NOT EXISTS ");
        let safe = safe.replace("CREATE INDEX ", "CREATE INDEX IF NOT EXISTS ");
        if let Err(e) = dst.execute_batch(&safe) {
            crate::log::debug("migrate", &format!("clone_schema skip: {}", e));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// Baseline SQL must be valid and create all expected tables.
    #[test]
    fn baseline_creates_all_tables() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(MIGRATIONS[0].sql)?;

        let expected_tables = [
            "sdk_sessions",
            "observations",
            "session_summaries",
            "pending_observations",
            "memories",
            "events",
            "entities",
            "memory_entities",
            "summarize_cooldown",
            "summarize_locks",
            "ai_usage_events",
            "jobs",
            "workstreams",
            "workstream_sessions",
        ];
        for table in &expected_tables {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            assert!(exists, "table {} not created by baseline", table);
        }
        Ok(())
    }

    /// ALTER TABLE in migration SQL must use constant DEFAULT values only.
    /// SQLite rejects non-constant expressions in ALTER TABLE ADD COLUMN.
    #[test]
    fn migration_sql_has_no_nonconstant_alter_defaults() {
        for m in MIGRATIONS {
            for line in m.sql.lines() {
                let upper = line.trim().to_uppercase();
                assert!(
                    !(upper.starts_with("ALTER TABLE") && upper.contains("DEFAULT (")),
                    "v{:03}_{} has non-constant DEFAULT in ALTER TABLE: {}",
                    m.version,
                    m.name,
                    line.trim()
                );
            }
        }
    }

    /// Full migration run on empty DB must succeed and track correctly.
    #[test]
    fn full_migration_on_empty_db() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        run_migrations(&conn)?;

        let applied = applied_versions(&conn)?;
        assert_eq!(applied, vec![1], "baseline should be applied");

        let uv: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        assert_eq!(uv, 13, "user_version should be 13 after baseline");
        Ok(())
    }

    /// Existing DB at user_version=13 should skip baseline and transition.
    #[test]
    fn transition_from_old_system_skips_baseline() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA user_version = 13;")?;
        conn.execute_batch(MIGRATIONS[0].sql)?; // tables exist

        run_migrations(&conn)?;

        let applied = applied_versions(&conn)?;
        assert_eq!(applied, vec![1], "baseline should be marked applied");
        Ok(())
    }

    /// DB at old version (< 13) should be rejected with clear error.
    #[test]
    fn rejects_old_schema_version() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA user_version = 10;")?;
        conn.execute_batch("CREATE TABLE observations (id INTEGER PRIMARY KEY);")?;

        let result = run_migrations(&conn);
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("v0.3.7"),
            "error should mention v0.3.7: {}",
            err
        );
        Ok(())
    }
}
