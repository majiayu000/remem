use anyhow::{Context, Result};
use rusqlite::Connection;

use super::SchemaInvariant;

mod shape;
use shape::{require_create_sql, require_foreign_key, require_unique_columns};

const V084_MIGRATION_SQL: &str = include_str!("../../../migrations/v084_session_observatory.sql");

pub(in crate::migrate) const V084_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(84, "session_observatory", "session_turns"),
    SchemaInvariant::table(84, "session_observatory", "session_turn_actions"),
    SchemaInvariant::index(84, "session_observatory", "idx_session_turns_recent"),
    SchemaInvariant::index(
        84,
        "session_observatory",
        "idx_session_turns_project_recent",
    ),
    SchemaInvariant::index(84, "session_observatory", "idx_session_turns_session"),
    SchemaInvariant::index(84, "session_observatory", "idx_session_turns_identity"),
    SchemaInvariant::index(84, "session_observatory", "idx_session_turn_actions_turn"),
    SchemaInvariant::index(84, "session_observatory", "idx_session_turn_actions_event"),
    SchemaInvariant::index(
        84,
        "session_observatory",
        "idx_raw_messages_activity_tuple_recent",
    ),
    SchemaInvariant::index(
        84,
        "session_observatory",
        "idx_raw_messages_activity_recent",
    ),
    SchemaInvariant::index(
        84,
        "session_observatory",
        "idx_raw_messages_project_activity_recent",
    ),
];

pub(in crate::migrate) fn v084_critical_shape_findings(conn: &Connection) -> Result<Vec<String>> {
    let mut findings = Vec::new();
    for table in ["session_turns", "session_turn_actions"] {
        require_create_sql(
            conn,
            &mut findings,
            "table",
            table,
            canonical_statement("CREATE TABLE", table)?,
        )?;
    }
    for index in [
        "idx_session_turns_recent",
        "idx_session_turns_project_recent",
        "idx_session_turns_session",
        "idx_session_turns_identity",
        "idx_session_turn_actions_turn",
        "idx_session_turn_actions_event",
        "idx_raw_messages_activity_tuple_recent",
        "idx_raw_messages_activity_recent",
        "idx_raw_messages_project_activity_recent",
    ] {
        require_create_sql(
            conn,
            &mut findings,
            "index",
            index,
            canonical_statement("CREATE INDEX", index)?,
        )?;
    }
    require_unique_columns(
        conn,
        &mut findings,
        "session_turns",
        &["source_root", "project", "session_id", "turn_index"],
    )?;
    require_unique_columns(
        conn,
        &mut findings,
        "session_turn_actions",
        &["session_turn_id", "action_index"],
    )?;

    for (table, from, target, to, on_delete) in [
        (
            "session_turns",
            "transcript_identity_id",
            "raw_session_identities",
            "id",
            "RESTRICT",
        ),
        (
            "session_turns",
            "session_row_id",
            "sessions",
            "id",
            "SET NULL",
        ),
        (
            "session_turns",
            "user_message_id",
            "raw_messages",
            "id",
            "CASCADE",
        ),
        (
            "session_turns",
            "understanding_message_id",
            "raw_messages",
            "id",
            "SET NULL",
        ),
        (
            "session_turns",
            "result_message_id",
            "raw_messages",
            "id",
            "SET NULL",
        ),
        (
            "session_turn_actions",
            "session_turn_id",
            "session_turns",
            "id",
            "CASCADE",
        ),
        (
            "session_turn_actions",
            "event_row_id",
            "captured_events",
            "id",
            "SET NULL",
        ),
    ] {
        require_foreign_key(conn, &mut findings, table, from, target, to, on_delete)?;
    }

    Ok(findings)
}

fn canonical_statement(object_type: &str, name: &str) -> Result<&'static str> {
    let prefix = format!("{object_type} {name}");
    V084_MIGRATION_SQL
        .split(';')
        .map(str::trim)
        .find_map(|statement| statement.find(&prefix).map(|offset| &statement[offset..]))
        .with_context(|| format!("v084 migration is missing canonical {object_type} {name}"))
}
