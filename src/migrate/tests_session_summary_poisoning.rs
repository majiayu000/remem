//! v072 session summary poisoning migration tests (GH-855).

use anyhow::Result;
use rusqlite::{params, Connection};

use super::MIGRATIONS;

fn conn_with_migrations_below(version: i64) -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version < version)
    {
        conn.execute_batch(migration.sql)?;
    }
    Ok(conn)
}

fn v072_sql() -> &'static str {
    MIGRATIONS
        .iter()
        .find(|migration| migration.version == 73)
        .expect("v072 migration must be registered")
        .sql
}

#[test]
fn migration_marks_pre_v072_summaries_legacy_unscanned() -> Result<()> {
    let conn = conn_with_migrations_below(73)?;
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at_epoch)
         VALUES ('mem-legacy', '/repo', 'old request', 'old completion', 100)",
        [],
    )?;

    conn.execute_batch(v072_sql())?;

    let (status, blocks): (String, i64) = conn.query_row(
        "SELECT poisoning_status, poisoning_block_count
         FROM session_summaries WHERE memory_session_id = 'mem-legacy'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(status, "legacy_unscanned");
    assert_eq!(blocks, 0);
    Ok(())
}

#[test]
fn post_migration_insert_defaults_to_safe_and_rejects_unknown_status() -> Result<()> {
    let conn = conn_with_migrations_below(i64::MAX)?;
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at_epoch)
         VALUES ('mem-new', '/repo', 'new request', 'done', 200)",
        [],
    )?;
    let status: String = conn.query_row(
        "SELECT poisoning_status FROM session_summaries WHERE memory_session_id = 'mem-new'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(status, "safe");

    let invalid = conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at_epoch, poisoning_status)
         VALUES ('mem-bad', '/repo', 'x', 'y', 300, 'not_a_status')",
        [],
    );
    assert!(invalid.is_err(), "closed status set must be enforced");
    Ok(())
}

#[test]
fn migration_remains_registered_after_later_schema_versions() {
    let migration = super::types::MIGRATIONS
        .iter()
        .find(|migration| migration.version == 73)
        .expect("v073 migration must remain registered");
    assert_eq!(migration.name, "session_summary_poisoning");
}

#[test]
fn quarantine_metadata_round_trips() -> Result<()> {
    let conn = conn_with_migrations_below(i64::MAX)?;
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at_epoch,
          poisoning_status, quarantine_stage, quarantine_field,
          quarantine_event_id, quarantine_pattern_id, quarantine_pattern_version)
         VALUES ('mem-quar', '/repo', 'x', 'y', 400,
                 'quarantined', 'source', 'source_event', 7,
                 'override_previous_instructions', 1)",
        [],
    )?;
    let (stage, field, event_id, pattern, version): (String, String, i64, String, i64) = conn
        .query_row(
            "SELECT quarantine_stage, quarantine_field, quarantine_event_id,
                    quarantine_pattern_id, quarantine_pattern_version
             FROM session_summaries WHERE memory_session_id = 'mem-quar'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
    assert_eq!(stage, "source");
    assert_eq!(field, "source_event");
    assert_eq!(event_id, 7);
    assert_eq!(pattern, "override_previous_instructions");
    assert_eq!(version, 1);

    let indexed: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'index' AND name = 'idx_session_summaries_poisoning'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(indexed, 1);
    conn.execute(
        "UPDATE session_summaries SET poisoning_status = 'acknowledged',
                acknowledged_pattern_id = 'override_previous_instructions',
                acknowledged_pattern_version = 1,
                acknowledged_at_epoch = 500
         WHERE memory_session_id = 'mem-quar'",
        params![],
    )?;
    Ok(())
}
