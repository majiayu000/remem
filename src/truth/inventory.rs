//! Aggregate-only, snapshot-bound inventory over the G2 visibility projection.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::visibility::classify_memory;

pub const MEMORY_VISIBILITY_INVENTORY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryVisibilityInventory {
    pub schema_version: u32,
    pub runtime_version: String,
    pub database_schema_version: i64,
    pub sqlite_user_version: i64,
    pub as_of_epoch: i64,
    pub snapshot_memory_count: u64,
    pub snapshot_max_memory_id: Option<i64>,
    pub snapshot_max_updated_at_epoch: Option<i64>,
    pub classification_counts: BTreeMap<String, u64>,
    pub reason_counts: BTreeMap<String, u64>,
    pub inventory_sha256: String,
}

pub fn build_memory_visibility_inventory(
    conn: &Connection,
    as_of_epoch: i64,
) -> Result<MemoryVisibilityInventory> {
    let snapshot = conn.unchecked_transaction()?;
    let report = build_memory_visibility_inventory_in_snapshot(&snapshot, as_of_epoch)?;
    snapshot.commit()?;
    Ok(report)
}

fn build_memory_visibility_inventory_in_snapshot(
    conn: &Connection,
    as_of_epoch: i64,
) -> Result<MemoryVisibilityInventory> {
    let sqlite_user_version = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let database_schema_version = logical_schema_version(conn, sqlite_user_version)?;
    let (snapshot_memory_count, snapshot_max_memory_id, snapshot_max_updated_at_epoch) = conn
        .query_row(
            "SELECT COUNT(*), MAX(id), MAX(updated_at_epoch) FROM memories",
            [],
            |row| Ok((row.get::<_, u64>(0)?, row.get(1)?, row.get(2)?)),
        )
        .context("bind memory visibility inventory snapshot")?;
    let mut statement = conn.prepare("SELECT id FROM memories ORDER BY id")?;
    let ids = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut classification_counts = BTreeMap::new();
    let mut reason_counts = BTreeMap::new();
    for id in ids {
        let visibility = classify_memory(conn, id, as_of_epoch)?;
        *classification_counts
            .entry(visibility.classification.as_str().to_string())
            .or_insert(0) += 1;
        *reason_counts
            .entry(visibility.reason.as_str().to_string())
            .or_insert(0) += 1;
    }
    let mut report = MemoryVisibilityInventory {
        schema_version: MEMORY_VISIBILITY_INVENTORY_SCHEMA_VERSION,
        runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        database_schema_version,
        sqlite_user_version,
        as_of_epoch,
        snapshot_memory_count,
        snapshot_max_memory_id,
        snapshot_max_updated_at_epoch,
        classification_counts,
        reason_counts,
        inventory_sha256: String::new(),
    };
    report.inventory_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&report)?));
    Ok(report)
}

fn logical_schema_version(conn: &Connection, sqlite_user_version: i64) -> Result<i64> {
    let has_ledger: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema
                       WHERE type='table' AND name='_schema_migrations')",
        [],
        |row| row.get(0),
    )?;
    if !has_ledger {
        return Ok(sqlite_user_version);
    }
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM _schema_migrations",
        [],
        |row| row.get(0),
    )
    .context("read logical schema version")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn inventory_is_deterministic_content_free_and_snapshot_bound() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::memory::tests_helper::setup_memory_schema(&conn);
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, source_trust_class)
             VALUES (1, '/repo', 'SECRET_TITLE', 'SECRET_BODY', 'bugfix',
                     10, 11, 'active', 'local_tool_output')",
            [],
        )?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, source_trust_class)
             VALUES (2, '/repo', 'manual', 'safe', 'bugfix',
                     12, 13, 'active', 'user_prompt')",
            [],
        )?;
        let first = build_memory_visibility_inventory(&conn, 100)?;
        let second = build_memory_visibility_inventory(&conn, 100)?;
        assert_eq!(first, second);
        assert_eq!(first.snapshot_memory_count, 2);
        assert_eq!(first.classification_counts["legacy_unverified"], 1);
        assert_eq!(first.classification_counts["current"], 1);
        let json = serde_json::to_string(&first)?;
        assert!(!json.contains("SECRET_TITLE"));
        assert!(!json.contains("SECRET_BODY"));

        conn.execute("UPDATE memories SET updated_at_epoch = 14 WHERE id = 2", [])?;
        let changed = build_memory_visibility_inventory(&conn, 100)?;
        assert_ne!(first.inventory_sha256, changed.inventory_sha256);
        Ok(())
    }

    #[test]
    fn inventory_snapshot_stays_consistent_after_wal_writer_commit() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "remem-g2-inventory-{}-{}.db",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        let result = (|| -> Result<()> {
            let reader = Connection::open(&path)?;
            reader.execute_batch("PRAGMA journal_mode = WAL;")?;
            crate::memory::tests_helper::setup_memory_schema(&reader);
            reader.execute(
                "INSERT INTO memories
                 (id, project, title, content, memory_type, created_at_epoch,
                  updated_at_epoch, status, source_trust_class)
                 VALUES (1, '/repo', 'before', 'body', 'bugfix', 10, 10,
                         'active', 'local_tool_output')",
                [],
            )?;

            let writer = Connection::open(&path)?;
            writer.execute_batch("PRAGMA journal_mode = WAL;")?;
            let snapshot = reader.unchecked_transaction()?;
            let _: i64 =
                snapshot.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
            writer.execute(
                "INSERT INTO memories
                 (id, project, title, content, memory_type, created_at_epoch,
                  updated_at_epoch, status, source_trust_class)
                 VALUES (2, '/repo', 'after', 'body', 'bugfix', 20, 20,
                         'active', 'local_tool_output')",
                [],
            )?;

            let report = build_memory_visibility_inventory_in_snapshot(&snapshot, 100)?;
            assert_eq!(report.snapshot_memory_count, 1);
            assert_eq!(report.snapshot_max_memory_id, Some(1));
            assert_eq!(report.classification_counts["legacy_unverified"], 1);
            snapshot.commit()?;
            Ok(())
        })();
        let _ = std::fs::remove_file(&path);
        result
    }
}
