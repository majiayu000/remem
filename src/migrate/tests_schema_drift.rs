use anyhow::Result;
use rusqlite::{params, Connection};

use super::{dry_run_pending, run_migrations, MIGRATIONS};

#[test]
fn run_migrations_repairs_v022_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_with_v022_missing_objects(&conn)?;

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
    create_current_schema_with_v022_missing_objects(&conn)?;

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

fn create_current_schema_with_v022_missing_objects(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=OFF;")?;
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version != 22)
    {
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
