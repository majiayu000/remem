//! v2 / v1 schema detection gate.
//!
//! v2 lives in `~/.remem/v2.sqlite`, but the same `remem` binary still opens
//! the legacy `~/.remem/remem.db` for read-only commands. Per
//! SPEC-memory-system-v2.1-revisions §2 S4, write commands must refuse a
//! legacy v1 connection with a fixed user-facing message; read-only commands
//! remain available. This module is the single source of truth for
//! distinguishing the two database states. Wiring into the CLI router lands
//! in Milestone A.4.

use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;

const V2_MARKER_TABLE: &str = "hosts";
const V1_MARKER_TABLE: &str = "sdk_sessions";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbKind {
    Empty,
    V1Legacy { user_version: i64 },
    V2 { user_version: i64 },
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
    let has_v2 = table_exists(conn, V2_MARKER_TABLE)?;
    let has_v1 = table_exists(conn, V1_MARKER_TABLE)?;
    match (has_v2, has_v1) {
        (true, false) => Ok(DbKind::V2 { user_version }),
        (false, true) => Ok(DbKind::V1Legacy { user_version }),
        (false, false) => Ok(DbKind::Empty),
        (true, true) => Err(anyhow!(
            "DB has both v1 (sdk_sessions) and v2 (hosts) tables; \
             refusing to operate on a mixed-schema database"
        )),
    }
}

/// Gate for write-side commands. Empty / V2 pass; V1Legacy returns the fixed
/// user-facing message from v2.1 §2 S4 so every refusal site emits the same
/// guidance.
pub fn refuse_v1_for_writes(conn: &Connection) -> Result<()> {
    match detect_db_kind(conn)? {
        DbKind::V2 { .. } | DbKind::Empty => Ok(()),
        DbKind::V1Legacy { user_version } => Err(anyhow!(legacy_refusal_message(user_version))),
    }
}

pub fn legacy_refusal_message(user_version: i64) -> String {
    format!(
        "Legacy v1 schema detected (user_version={user_version}).\n\
         Run `remem admin backup` then `remem admin reset-v2 --confirm-destructive` to upgrade.\n\
         Read-only commands remain available."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_in_memory() -> Connection {
        Connection::open_in_memory().expect("open in-memory sqlite")
    }

    fn install_v1_marker(conn: &Connection, user_version: i64) {
        conn.execute_batch(&format!(
            "CREATE TABLE sdk_sessions(id INTEGER PRIMARY KEY); \
             PRAGMA user_version = {user_version};"
        ))
        .expect("install v1 marker");
    }

    fn install_v2_marker(conn: &Connection, user_version: i64) {
        conn.execute_batch(&format!(
            "CREATE TABLE hosts(id INTEGER PRIMARY KEY); \
             PRAGMA user_version = {user_version};"
        ))
        .expect("install v2 marker");
    }

    #[test]
    fn empty_db_is_classified_as_empty() {
        let conn = open_in_memory();
        assert_eq!(detect_db_kind(&conn).unwrap(), DbKind::Empty);
    }

    #[test]
    fn v1_baseline_is_classified_as_v1_legacy() {
        let conn = open_in_memory();
        install_v1_marker(&conn, 13);
        assert_eq!(
            detect_db_kind(&conn).unwrap(),
            DbKind::V1Legacy { user_version: 13 }
        );
    }

    #[test]
    fn v2_baseline_is_classified_as_v2() {
        let conn = open_in_memory();
        install_v2_marker(&conn, 1);
        assert_eq!(
            detect_db_kind(&conn).unwrap(),
            DbKind::V2 { user_version: 1 }
        );
    }

    #[test]
    fn mixed_v1_and_v2_tables_returns_error() {
        let conn = open_in_memory();
        install_v1_marker(&conn, 13);
        // install_v2_marker would reset user_version; here we only need the table.
        conn.execute_batch("CREATE TABLE hosts(id INTEGER PRIMARY KEY);")
            .unwrap();
        let err = detect_db_kind(&conn).unwrap_err().to_string();
        assert!(err.contains("mixed-schema"), "got: {err}");
    }

    #[test]
    fn refuse_passes_on_v2() {
        let conn = open_in_memory();
        install_v2_marker(&conn, 1);
        refuse_v1_for_writes(&conn).expect("v2 must pass");
    }

    #[test]
    fn refuse_passes_on_empty() {
        let conn = open_in_memory();
        refuse_v1_for_writes(&conn).expect("empty must pass; caller will init");
    }

    #[test]
    fn refuse_blocks_v1_with_fixed_message() {
        let conn = open_in_memory();
        install_v1_marker(&conn, 4);
        let err = refuse_v1_for_writes(&conn).unwrap_err().to_string();
        assert!(err.contains("Legacy v1 schema detected"), "got: {err}");
        assert!(err.contains("user_version=4"), "got: {err}");
        assert!(err.contains("remem admin backup"), "got: {err}");
        assert!(err.contains("remem admin reset-v2"), "got: {err}");
        assert!(err.contains("Read-only commands remain available"), "got: {err}");
    }

    #[test]
    fn legacy_refusal_message_is_deterministic() {
        let m1 = legacy_refusal_message(4);
        let m2 = legacy_refusal_message(4);
        assert_eq!(m1, m2);
        assert!(m1.contains("user_version=4"));
    }
}
