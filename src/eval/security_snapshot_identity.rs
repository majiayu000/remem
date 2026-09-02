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
            let value = row.get_ref(index)?;
            if volatile_generation_field(table, column) {
                validate_volatile_timestamp(value, table, column)?;
                encoded.push(0xff);
                continue;
            }
            encode_value(value, &mut encoded);
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

fn validate_volatile_timestamp(value: ValueRef<'_>, table: &str, column: &str) -> Result<()> {
    const MAX_EPOCH_SECONDS_EXCLUSIVE: i64 = 10_000_000_000;
    let valid = match value {
        ValueRef::Null => true,
        ValueRef::Integer(epoch) => (0..MAX_EPOCH_SECONDS_EXCLUSIVE).contains(&epoch),
        ValueRef::Text(bytes) if column == "created_at" => std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
            .is_some_and(|timestamp| {
                (0..MAX_EPOCH_SECONDS_EXCLUSIVE).contains(&timestamp.timestamp())
            }),
        ValueRef::Real(_) | ValueRef::Text(_) | ValueRef::Blob(_) => false,
    };
    ensure!(
        valid,
        "invalid volatile timestamp {table}.{column}: expected null, epoch seconds in [0, {MAX_EPOCH_SECONDS_EXCLUSIVE}), or RFC3339 created_at text"
    );
    Ok(())
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

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::{params, Connection};

    use super::snapshot_identity;

    fn volatile_timestamp_connection(value: rusqlite::types::Value) -> Result<Connection> {
        let connection = Connection::open_in_memory()?;
        connection.execute("CREATE TABLE hosts (created_at_epoch)", [])?;
        connection.execute(
            "INSERT INTO hosts (created_at_epoch) VALUES (?1)",
            params![value],
        )?;
        Ok(connection)
    }

    #[test]
    fn snapshot_identity_rejects_malformed_volatile_timestamp_values() -> Result<()> {
        for invalid in [
            rusqlite::types::Value::Blob(b"private hidden bytes".to_vec()),
            rusqlite::types::Value::Text("private hidden text".to_string()),
            rusqlite::types::Value::Integer(-1),
            rusqlite::types::Value::Integer(10_000_000_000),
        ] {
            let error = snapshot_identity(&volatile_timestamp_connection(invalid)?)
                .expect_err("malformed volatile timestamp must fail closed");
            assert!(
                format!("{error:#}").contains("invalid volatile timestamp hosts.created_at_epoch"),
                "unexpected diagnostic: {error:#}"
            );
        }
        Ok(())
    }

    #[test]
    fn snapshot_identity_normalizes_valid_volatile_timestamps() -> Result<()> {
        let first = snapshot_identity(&volatile_timestamp_connection(
            rusqlite::types::Value::Integer(1_700_000_000),
        )?)?;
        let second = snapshot_identity(&volatile_timestamp_connection(
            rusqlite::types::Value::Integer(1_800_000_000),
        )?)?;

        assert_eq!(first, second);
        Ok(())
    }

    #[test]
    fn snapshot_identity_preserves_memory_recency_timestamps() -> Result<()> {
        fn connection(updated_at_epoch: i64) -> Result<Connection> {
            let connection = Connection::open_in_memory()?;
            connection.execute(
                "CREATE TABLE memories (created_at_epoch INTEGER, updated_at_epoch INTEGER)",
                [],
            )?;
            connection.execute(
                "INSERT INTO memories (created_at_epoch, updated_at_epoch) VALUES (1, ?1)",
                [updated_at_epoch],
            )?;
            Ok(connection)
        }

        let first = snapshot_identity(&connection(1_700_000_000)?)?;
        let second = snapshot_identity(&connection(1_800_000_000)?)?;

        assert_ne!(first, second);
        Ok(())
    }
}
