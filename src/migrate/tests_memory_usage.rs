use anyhow::Result;
use rusqlite::Connection;

use super::MIGRATIONS;

#[test]
fn memory_usage_migration_adds_columns_with_defaults() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    for migration in &MIGRATIONS[..44] {
        conn.execute_batch(migration.sql)?;
    }

    conn.execute(
        "INSERT INTO memories(project, title, content, memory_type, created_at_epoch, updated_at_epoch)
         VALUES ('proj', 'Usage target', 'body', 'decision', 100, 100)",
        [],
    )?;
    conn.execute_batch(MIGRATIONS[44].sql)?;

    let usage: (i64, Option<i64>) = conn.query_row(
        "SELECT access_count, last_accessed_epoch FROM memories WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(usage, (0, None));
    for name in [
        "memory_citation_events",
        "memory_usage_events",
        "idx_memories_usage",
        "idx_memory_citation_events_project_recent",
        "idx_memory_usage_events_memory_recent",
    ] {
        let exists: bool = conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE name = ?1",
            [name],
            |_| Ok(true),
        )?;
        assert!(exists, "{name} should exist");
    }
    Ok(())
}
