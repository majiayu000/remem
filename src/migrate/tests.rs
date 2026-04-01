use anyhow::Result;
use rusqlite::Connection;

use super::state::applied_versions;
use super::{dry_run_pending, run_migrations, MIGRATIONS};

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

#[test]
fn migration_sql_has_no_nonconstant_alter_defaults() {
    for migration in MIGRATIONS {
        for line in migration.sql.lines() {
            let upper = line.trim().to_uppercase();
            assert!(
                !(upper.starts_with("ALTER TABLE") && upper.contains("DEFAULT (")),
                "v{:03}_{} has non-constant DEFAULT in ALTER TABLE: {}",
                migration.version,
                migration.name,
                line.trim()
            );
        }
    }
}

#[test]
fn full_migration_on_empty_db() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    let applied = applied_versions(&conn)?;
    assert_eq!(applied, vec![1]);

    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    assert_eq!(user_version, 13);
    Ok(())
}

#[test]
fn transition_from_old_system_skips_baseline() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA user_version = 13;")?;
    conn.execute_batch(MIGRATIONS[0].sql)?;

    run_migrations(&conn)?;

    let applied = applied_versions(&conn)?;
    assert_eq!(applied, vec![1]);
    Ok(())
}

#[test]
fn rejects_old_schema_version() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA user_version = 10;")?;
    conn.execute_batch("CREATE TABLE observations (id INTEGER PRIMARY KEY);")?;

    let result = run_migrations(&conn);
    assert!(result.is_err());
    let error = format!("{}", result.unwrap_err());
    assert!(
        error.contains("v0.3.7"),
        "error should mention v0.3.7: {}",
        error
    );
    Ok(())
}

#[test]
fn dry_run_pending_reports_no_pending_for_current_schema() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    assert!(result.error.is_none());
    Ok(())
}

#[test]
fn dry_run_pending_reports_pending_for_new_db() -> Result<()> {
    let conn = Connection::open_in_memory()?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.current_version, 0);
    assert_eq!(result.pending_count, 1);
    assert!(result.error.is_none());
    Ok(())
}
