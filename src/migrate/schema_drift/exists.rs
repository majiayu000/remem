use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use super::SchemaObject;

pub(super) fn schema_object_exists(conn: &Connection, object: SchemaObject) -> Result<bool> {
    match object {
        SchemaObject::Table(table) => table_exists(conn, table),
        SchemaObject::Column { table, column } => column_exists(conn, table, column),
        SchemaObject::Index(index) => index_exists(conn, index),
        SchemaObject::Trigger(trigger) => trigger_exists(conn, trigger),
    }
}

pub(super) fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn trigger_exists(conn: &Connection, trigger: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='trigger' AND name=?1",
            [trigger],
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
