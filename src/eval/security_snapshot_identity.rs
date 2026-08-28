use std::collections::BTreeMap;

use anyhow::{ensure, Context, Result};
use rusqlite::{types::ValueRef, Connection};
use sha2::{Digest, Sha256};

pub(crate) type SnapshotIdentity = BTreeMap<String, String>;

pub(crate) fn snapshot_identity(connection: &Connection) -> Result<SnapshotIdentity> {
    let mut identity = BTreeMap::new();
    identity.insert("sqlite_schema".to_string(), schema_identity(connection)?);
    for table in table_names(connection)? {
        ensure_safe_identifier(&table)?;
        identity.insert(table.clone(), table_identity(connection, &table)?);
    }
    Ok(identity)
}

fn schema_identity(connection: &Connection) -> Result<String> {
    let mut statement = connection.prepare(
        "SELECT type, name, tbl_name, sql
           FROM sqlite_schema
          ORDER BY type, name, tbl_name, COALESCE(sql, '')",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut hasher = Sha256::new();
    for row in rows {
        let (kind, name, table, sql) = row?;
        hash_bytes(&mut hasher, kind.as_bytes());
        hash_bytes(&mut hasher, name.as_bytes());
        hash_bytes(&mut hasher, table.as_bytes());
        hash_bytes(&mut hasher, sql.as_deref().unwrap_or_default().as_bytes());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn table_names(connection: &Connection) -> Result<Vec<String>> {
    let mut statement =
        connection.prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")?;
    let tables = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()
        .context("read complete snapshot table inventory")?;
    Ok(tables)
}

fn table_identity(connection: &Connection, table: &str) -> Result<String> {
    let mut statement = connection
        .prepare(&format!("SELECT * FROM \"{table}\""))
        .with_context(|| format!("read complete snapshot table {table}"))?;
    let columns = statement
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut rows = statement.query([])?;
    let mut encoded_rows = Vec::new();
    while let Some(row) = rows.next()? {
        let mut encoded = Vec::new();
        for (index, column) in columns.iter().enumerate() {
            if volatile_generation_field(table, column) {
                encoded.push(0xff);
                continue;
            }
            encode_value(row.get_ref(index)?, &mut encoded);
        }
        encoded_rows.push(encoded);
    }
    encoded_rows.sort();

    let mut hasher = Sha256::new();
    for column in &columns {
        hash_bytes(&mut hasher, column.as_bytes());
    }
    for row in &encoded_rows {
        hash_bytes(&mut hasher, row);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn encode_value(value: ValueRef<'_>, output: &mut Vec<u8>) {
    match value {
        ValueRef::Null => output.push(0),
        ValueRef::Integer(value) => {
            output.push(1);
            output.extend_from_slice(&value.to_be_bytes());
        }
        ValueRef::Real(value) => {
            output.push(2);
            output.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        ValueRef::Text(value) => {
            output.push(3);
            append_bytes(output, value);
        }
        ValueRef::Blob(value) => {
            output.push(4);
            append_bytes(output, value);
        }
    }
}

fn append_bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_be_bytes());
    output.extend_from_slice(value);
}

fn hash_bytes(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn ensure_safe_identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "snapshot table name is not a safe SQLite identifier: {value:?}"
    );
    Ok(())
}

fn volatile_generation_field(table: &str, column: &str) -> bool {
    matches!(
        (table, column),
        ("_schema_migrations", "applied_at_epoch")
            | ("captured_events", "inserted_at_epoch")
            | ("entities", "created_at_epoch")
            | ("extraction_tasks", "created_at_epoch" | "updated_at_epoch")
            | ("hosts", "created_at_epoch")
            | (
                "legacy_surface_state",
                "exhausted_at_epoch" | "updated_at_epoch"
            )
            | ("memories", "created_at_epoch" | "updated_at_epoch")
            | ("memory_activation_requests", "created_at_epoch")
            | ("memory_candidates", "created_at_epoch" | "updated_at_epoch")
            | ("memory_edges", "created_at_epoch")
            | ("memory_embeddings", "updated_at_epoch")
            | (
                "memory_facts",
                "created_at_epoch" | "learned_at_epoch" | "updated_at_epoch"
            )
            | ("memory_operation_log", "created_at_epoch")
            | ("memory_state_keys", "created_at_epoch" | "updated_at_epoch")
            | ("observations", "created_at" | "created_at_epoch")
            | ("projects", "created_at_epoch" | "updated_at_epoch")
            | ("retrieval_enrichment_compatibility", "updated_at_epoch")
            | ("sessions", "last_seen_at_epoch" | "started_at_epoch")
            | ("workspaces", "created_at_epoch" | "updated_at_epoch")
    )
}
