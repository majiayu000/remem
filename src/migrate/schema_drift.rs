use anyhow::{bail, Context, Result};
use rusqlite::Connection;

use super::state::{applied_versions, has_migration_table};
use super::transition::add_column_if_missing;

mod exists;

use exists::{schema_object_exists, table_exists};

const V022: i64 = 22;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SchemaObject {
    Table(&'static str),
    Column {
        table: &'static str,
        column: &'static str,
    },
    Index(&'static str),
    Trigger(&'static str),
}

impl SchemaObject {
    fn describe(self) -> String {
        match self {
            SchemaObject::Table(table) => format!("table {table}"),
            SchemaObject::Column { table, column } => format!("column {table}.{column}"),
            SchemaObject::Index(index) => format!("index {index}"),
            SchemaObject::Trigger(trigger) => format!("trigger {trigger}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SchemaInvariant {
    pub version: i64,
    pub migration: &'static str,
    pub object: SchemaObject,
}

impl SchemaInvariant {
    const fn table(version: i64, migration: &'static str, name: &'static str) -> Self {
        Self {
            version,
            migration,
            object: SchemaObject::Table(name),
        }
    }

    const fn column(
        version: i64,
        migration: &'static str,
        table: &'static str,
        column: &'static str,
    ) -> Self {
        Self {
            version,
            migration,
            object: SchemaObject::Column { table, column },
        }
    }

    const fn index(version: i64, migration: &'static str, name: &'static str) -> Self {
        Self {
            version,
            migration,
            object: SchemaObject::Index(name),
        }
    }

    const fn trigger(version: i64, migration: &'static str, name: &'static str) -> Self {
        Self {
            version,
            migration,
            object: SchemaObject::Trigger(name),
        }
    }

    fn label(self) -> String {
        format!("v{:03}_{}", self.version, self.migration)
    }
}

mod invariants;

pub(super) use invariants::{SCHEMA_INVARIANTS, V070_SCHEMA_INVARIANTS, V071_SCHEMA_INVARIANTS};

pub(crate) fn validate_schema_invariants(conn: &Connection) -> Result<Vec<String>> {
    if !has_migration_table(conn) {
        return Ok(Vec::new());
    }

    let applied = applied_versions(conn)?;
    missing_schema_invariants(conn, &applied)
}

pub(super) fn repair_known_schema_drift(conn: &Connection, applied: &[i64]) -> Result<Vec<String>> {
    let mut repaired = Vec::new();
    if applied.contains(&V022) {
        let missing = missing_v022_objects(conn)?;
        if !missing.is_empty() {
            repair_v022_memory_state_keys(conn)
                .context("repair v022_memory_state_keys schema drift")?;
            let still_missing = missing_v022_objects(conn)?;
            if !still_missing.is_empty() {
                bail!(
                    "repair v022_memory_state_keys schema drift incomplete: {}",
                    still_missing.join(", ")
                );
            }
            repaired.push(format!("v022_memory_state_keys ({})", missing.join(", ")));
        }
    }

    if applied.contains(&31) {
        let trigger = SchemaObject::Trigger("graph_edges_memory_state_keys_delete");
        if !schema_object_exists(conn, trigger)? {
            install_v031_state_delete_trigger(conn)
                .context("install v031 graph_edges memory_state_keys delete trigger")?;
            if schema_object_exists(conn, trigger)? {
                repaired.push(
                    "v031_graph_edges (trigger graph_edges_memory_state_keys_delete)".to_string(),
                );
            }
        }
    }

    let unresolved = missing_schema_invariants(conn, applied)?;
    if !unresolved.is_empty() {
        bail!(
            "schema drift requires manual repair: {}",
            unresolved.join("; ")
        );
    }
    Ok(repaired)
}

pub(super) fn install_v031_state_delete_trigger(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "graph_edges")? || !table_exists(conn, "memory_state_keys")? {
        return Ok(());
    }

    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS graph_edges_memory_state_keys_delete
        AFTER DELETE ON memory_state_keys
        BEGIN
            DELETE FROM graph_edges
            WHERE (from_node_kind = 'state' AND from_node_id = OLD.id)
               OR (to_node_kind = 'state' AND to_node_id = OLD.id);
        END;",
    )?;
    Ok(())
}

fn missing_schema_invariants(conn: &Connection, applied: &[i64]) -> Result<Vec<String>> {
    let mut missing = Vec::new();
    for invariant in SCHEMA_INVARIANTS
        .iter()
        .chain(V070_SCHEMA_INVARIANTS)
        .chain(V071_SCHEMA_INVARIANTS)
    {
        if !applied.contains(&invariant.version) || schema_object_exists(conn, invariant.object)? {
            continue;
        }
        missing.push(format!(
            "{} marked applied but missing {}",
            invariant.label(),
            invariant.object.describe()
        ));
    }
    Ok(missing)
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
    for invariant in SCHEMA_INVARIANTS
        .iter()
        .filter(|invariant| invariant.version == V022)
    {
        if !schema_object_exists(conn, invariant.object)? {
            missing.push(invariant.object.describe());
        }
    }
    Ok(missing)
}
