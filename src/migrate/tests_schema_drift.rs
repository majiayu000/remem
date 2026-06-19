use anyhow::Result;
use rusqlite::{params, Connection};

use super::{dry_run_pending, run_migrations, MIGRATIONS};

#[test]
fn validate_schema_invariants_is_clean_after_current_migrations() -> Result<()> {
    let conn = Connection::open_in_memory()?;

    run_migrations(&conn)?;

    let errors = super::validate_schema_invariants(&conn)?;
    assert!(errors.is_empty(), "unexpected schema drift: {errors:?}");
    Ok(())
}

#[test]
fn run_migrations_repairs_v022_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[22])?;

    run_migrations(&conn)?;

    assert!(conn
        .prepare("SELECT id, state_key FROM memory_state_keys LIMIT 0")
        .is_ok());
    assert!(conn
        .prepare("SELECT state_key_id FROM memories LIMIT 0")
        .is_ok());
    assert!(conn
        .prepare(
            "SELECT state_key, state_key_confidence, state_key_reason
             FROM memory_candidates LIMIT 0"
        )
        .is_ok());
    for index in [
        "idx_memory_state_keys_owner",
        "idx_memory_state_keys_current",
        "idx_memories_state_key_id",
        "idx_memory_candidates_state_key",
    ] {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1",
                [index],
                |_| Ok(true),
            )
            .unwrap_or(false);
        assert!(exists, "{index} index should be repaired");
    }
    Ok(())
}

#[test]
fn dry_run_pending_reports_v022_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[22])?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    let error = result
        .error
        .expect("schema drift must be reported even when no migrations are pending");
    assert!(error.contains("schema drift"));
    assert!(error.contains("v022_memory_state_keys marked applied"));
    assert!(error.contains("table memory_state_keys"));
    Ok(())
}

#[test]
fn dry_run_pending_reports_post_v022_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[45])?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    let error = result
        .error
        .expect("post-v022 schema drift must be reported when no migrations are pending");
    assert!(error.contains("schema drift"), "got: {error}");
    assert!(error.contains("v045_memory_usage_columns"), "got: {error}");
    assert!(
        error.contains("column memories.access_count"),
        "got: {error}"
    );
    assert!(
        error.contains("table memory_citation_events"),
        "got: {error}"
    );
    Ok(())
}

#[test]
fn run_migrations_rejects_post_v022_schema_drift_without_repairing() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[45])?;

    let error = run_migrations(&conn).expect_err("post-v022 drift must not be silently accepted");

    let message = format!("{error:#}");
    assert!(
        message.contains("schema drift requires manual repair"),
        "got: {message}"
    );
    assert!(
        message.contains("v045_memory_usage_columns"),
        "got: {message}"
    );
    let usage_table_exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name = 'memory_citation_events'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(usage_table_exists, 0);
    Ok(())
}

fn create_current_schema_missing_versions(
    conn: &Connection,
    missing_versions: &[i64],
) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=OFF;")?;
    for migration in MIGRATIONS.iter().filter(|migration| {
        !missing_versions
            .iter()
            .any(|missing| *missing == migration.version)
    }) {
        conn.execute_batch(migration.sql)?;
    }
    conn.execute_batch(
        "CREATE TABLE _schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at_epoch INTEGER NOT NULL
        );",
    )?;
    for migration in MIGRATIONS {
        conn.execute(
            "INSERT INTO _schema_migrations (version, name, applied_at_epoch)
             VALUES (?1, ?2, 1700000000)",
            params![migration.version, migration.name],
        )?;
    }
    conn.execute_batch(&format!(
        "PRAGMA user_version = {}; PRAGMA foreign_keys=ON;",
        super::types::OLD_BASELINE_VERSION - 1 + super::latest_schema_version()
    ))?;
    Ok(())
}
