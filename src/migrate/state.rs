use anyhow::Result;
use rusqlite::{params, Connection};

pub(super) fn ensure_migration_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at_epoch INTEGER NOT NULL
        )",
    )?;
    Ok(())
}

pub(super) fn has_migration_table(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='_schema_migrations'",
        [],
        |_| Ok(()),
    )
    .is_ok()
}

pub(super) fn applied_versions(conn: &Connection) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT version FROM _schema_migrations ORDER BY version")?;
    let versions: Vec<i64> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(versions)
}

pub(super) fn mark_applied(conn: &Connection, version: i64, name: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT OR IGNORE INTO _schema_migrations (version, name, applied_at_epoch)
         VALUES (?1, ?2, ?3)",
        params![version, name, now],
    )?;
    Ok(())
}
