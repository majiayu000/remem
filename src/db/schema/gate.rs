//! Schema detection gate for write-side commands.
//!
//! The same binary can still inspect older `remem.db` files for read-only
//! commands. Write commands must reject old-schema connections with one fixed
//! user-facing message.

use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;

const SCHEMA_MARKER_TABLE: &str = "hosts";
const OLD_SCHEMA_MARKER_TABLE: &str = "sdk_sessions";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbKind {
    Empty,
    Old { user_version: i64 },
    Schema { user_version: i64 },
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |row| row.get(0),
        )
        .with_context(|| format!("probe sqlite_master for table {name}"))?;
    Ok(count == 1)
}

pub fn detect_db_kind(conn: &Connection) -> Result<DbKind> {
    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);
    let has_schema = table_exists(conn, SCHEMA_MARKER_TABLE)?;
    let has_old_schema = table_exists(conn, OLD_SCHEMA_MARKER_TABLE)?;
    match (has_schema, has_old_schema) {
        (true, false) => Ok(DbKind::Schema { user_version }),
        (false, true) => Ok(DbKind::Old { user_version }),
        (false, false) => Ok(DbKind::Empty),
        (true, true) => Err(anyhow!(
            "DB has both old (sdk_sessions) and schema (hosts) tables; \
             refusing to operate on a mixed-schema database"
        )),
    }
}

/// Gate for write-side commands. Empty / Schema pass; Old returns the fixed
/// user-facing message so every refusal site emits the same guidance.
pub fn refuse_old_schema_for_writes(conn: &Connection) -> Result<()> {
    match detect_db_kind(conn)? {
        DbKind::Schema { .. } | DbKind::Empty => Ok(()),
        DbKind::Old { user_version } => Err(anyhow!(old_schema_refusal_message(user_version))),
    }
}

pub fn old_schema_refusal_message(user_version: i64) -> String {
    format!(
        "Old schema detected (user_version={user_version}).\n\
         Run `remem admin backup` then `remem admin reset-schema --confirm-destructive` to upgrade.\n\
         Read-only commands remain available."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_in_memory() -> Connection {
        Connection::open_in_memory().expect("open in-memory sqlite")
    }

    fn install_old_schema_marker(conn: &Connection, user_version: i64) {
        conn.execute_batch(&format!(
            "CREATE TABLE sdk_sessions(id INTEGER PRIMARY KEY); \
             PRAGMA user_version = {user_version};"
        ))
        .expect("install old schema marker");
    }

    fn install_schema_marker(conn: &Connection, user_version: i64) {
        conn.execute_batch(&format!(
            "CREATE TABLE hosts(id INTEGER PRIMARY KEY); \
             PRAGMA user_version = {user_version};"
        ))
        .expect("install schema marker");
    }

    #[test]
    fn empty_db_is_classified_as_empty() {
        let conn = open_in_memory();
        assert_eq!(detect_db_kind(&conn).unwrap(), DbKind::Empty);
    }

    #[test]
    fn old_baseline_is_classified_as_old() {
        let conn = open_in_memory();
        install_old_schema_marker(&conn, 13);
        assert_eq!(
            detect_db_kind(&conn).unwrap(),
            DbKind::Old { user_version: 13 }
        );
    }

    #[test]
    fn schema_baseline_is_classified_as_schema() {
        let conn = open_in_memory();
        install_schema_marker(&conn, 1);
        assert_eq!(
            detect_db_kind(&conn).unwrap(),
            DbKind::Schema { user_version: 1 }
        );
    }

    #[test]
    fn mixed_old_and_schema_tables_returns_error() {
        let conn = open_in_memory();
        install_old_schema_marker(&conn, 13);
        // install_schema_marker would reset user_version; here we only need the table.
        conn.execute_batch("CREATE TABLE hosts(id INTEGER PRIMARY KEY);")
            .unwrap();
        let err = detect_db_kind(&conn).unwrap_err().to_string();
        assert!(err.contains("mixed-schema"), "got: {err}");
    }

    #[test]
    fn refuse_passes_on_schema() {
        let conn = open_in_memory();
        install_schema_marker(&conn, 1);
        refuse_old_schema_for_writes(&conn).expect("schema must pass");
    }

    #[test]
    fn refuse_passes_on_empty() {
        let conn = open_in_memory();
        refuse_old_schema_for_writes(&conn).expect("empty must pass; caller will init");
    }

    #[test]
    fn refuse_blocks_old_schema_with_fixed_message() {
        let conn = open_in_memory();
        install_old_schema_marker(&conn, 4);
        let err = refuse_old_schema_for_writes(&conn).unwrap_err().to_string();
        assert!(err.contains("Old schema detected"), "got: {err}");
        assert!(err.contains("user_version=4"), "got: {err}");
        assert!(err.contains("remem admin backup"), "got: {err}");
        assert!(err.contains("remem admin reset-schema"), "got: {err}");
        assert!(
            err.contains("Read-only commands remain available"),
            "got: {err}"
        );
    }

    #[test]
    fn old_schema_refusal_message_is_deterministic() {
        let m1 = old_schema_refusal_message(4);
        let m2 = old_schema_refusal_message(4);
        assert_eq!(m1, m2);
        assert!(m1.contains("user_version=4"));
    }
}
