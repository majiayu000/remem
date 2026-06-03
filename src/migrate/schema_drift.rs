use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OptionalExtension};

use super::state::{applied_versions, has_migration_table};
use super::transition::add_column_if_missing;

const V022: i64 = 22;

pub(crate) fn validate_schema_invariants(conn: &Connection) -> Result<Vec<String>> {
    if !has_migration_table(conn) {
        return Ok(Vec::new());
    }

    let applied = applied_versions(conn)?;
    let mut errors = Vec::new();
    if applied.contains(&V022) {
        for missing in missing_v022_objects(conn)? {
            errors.push(format!(
                "v022_memory_state_keys marked applied but missing {missing}"
            ));
        }
    }
    Ok(errors)
}

pub(super) fn repair_known_schema_drift(conn: &Connection, applied: &[i64]) -> Result<Vec<String>> {
    let mut repaired = Vec::new();
    if !applied.contains(&V022) {
        return Ok(repaired);
    }

    let missing = missing_v022_objects(conn)?;
    if missing.is_empty() {
        return Ok(repaired);
    }

    repair_v022_memory_state_keys(conn).context("repair v022_memory_state_keys schema drift")?;
    let still_missing = missing_v022_objects(conn)?;
    if !still_missing.is_empty() {
        bail!(
            "repair v022_memory_state_keys schema drift incomplete: {}",
            still_missing.join(", ")
        );
    }

    repaired.push(format!("v022_memory_state_keys ({})", missing.join(", ")));
    Ok(repaired)
}

fn repair_v022_memory_state_keys(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_state_keys (
            id INTEGER PRIMARY KEY,
            owner_scope TEXT NOT NULL,
            owner_key TEXT NOT NULL,
            memory_type TEXT NOT NULL,
            state_key TEXT NOT NULL,
            state_label TEXT,
            state_status TEXT NOT NULL DEFAULT 'active',
            current_memory_id INTEGER,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            UNIQUE(owner_scope, owner_key, memory_type, state_key),
            FOREIGN KEY(current_memory_id) REFERENCES memories(id)
        );",
    )?;

    add_column_if_missing(conn, "memories", "state_key_id", "INTEGER")?;
    add_column_if_missing(conn, "memory_candidates", "state_key", "TEXT")?;
    add_column_if_missing(conn, "memory_candidates", "state_key_confidence", "REAL")?;
    add_column_if_missing(conn, "memory_candidates", "state_key_reason", "TEXT")?;

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memory_state_keys_owner
            ON memory_state_keys(owner_scope, owner_key, memory_type, state_status);
        CREATE INDEX IF NOT EXISTS idx_memory_state_keys_current
            ON memory_state_keys(current_memory_id)
            WHERE current_memory_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_memories_state_key_id
            ON memories(state_key_id)
            WHERE state_key_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_memory_candidates_state_key
            ON memory_candidates(owner_scope, owner_key, memory_type, state_key)
            WHERE state_key IS NOT NULL;",
    )?;
    Ok(())
}

fn missing_v022_objects(conn: &Connection) -> Result<Vec<String>> {
    let mut missing = Vec::new();
    if !table_exists(conn, "memory_state_keys")? {
        missing.push("table memory_state_keys".to_string());
    }
    for (table, column) in [
        ("memories", "state_key_id"),
        ("memory_candidates", "state_key"),
        ("memory_candidates", "state_key_confidence"),
        ("memory_candidates", "state_key_reason"),
    ] {
        if !column_exists(conn, table, column)? {
            missing.push(format!("column {table}.{column}"));
        }
    }
    for index in [
        "idx_memory_state_keys_owner",
        "idx_memory_state_keys_current",
        "idx_memories_state_key_id",
        "idx_memory_candidates_state_key",
    ] {
        if !index_exists(conn, index)? {
            missing.push(format!("index {index}"));
        }
    }
    Ok(missing)
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn index_exists(conn: &Connection, index: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1",
            [index],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let sql = format!("PRAGMA table_info({})", quote_identifier(table));
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
