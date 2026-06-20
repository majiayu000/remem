use anyhow::Result;
use rusqlite::{params, Connection};

use super::run_migrations;

#[test]
fn user_context_summary_migration_creates_rows_and_indexes() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    for name in [
        "user_context_claims",
        "user_context_summaries",
        "idx_user_context_summaries_owner_active",
        "idx_user_context_summaries_user_recent",
    ] {
        let exists: bool = conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE name = ?1",
            [name],
            |_| Ok(true),
        )?;
        assert!(exists, "{name} should exist after user-context migrations");
    }
    Ok(())
}

#[test]
fn user_context_summary_source_json_is_schema_validated() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    let err = insert_summary_row(&conn, "not-json", "[]", "[]")
        .expect_err("invalid source JSON should fail schema checks");
    assert!(err.to_string().contains("CHECK"));
    let err = insert_summary_row(&conn, "{}", "[]", "[]")
        .expect_err("source claim JSON must be an array");
    assert!(err.to_string().contains("CHECK"));
    let err = insert_summary_row(&conn, "[]", "{}", "[]")
        .expect_err("source memory JSON must be an array");
    assert!(err.to_string().contains("CHECK"));
    let err = insert_summary_row(&conn, "[]", "[]", "{}")
        .expect_err("source activity JSON must be an array");
    assert!(err.to_string().contains("CHECK"));
    Ok(())
}

fn insert_summary_row(
    conn: &Connection,
    source_claim_ids_json: &str,
    source_memory_ids_json: &str,
    source_activity_refs_json: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO user_context_summaries
         (user_key, owner_scope, owner_key, scope, scope_key, summary_text,
          source_claim_ids_json, source_memory_ids_json, source_activity_refs_json,
          status, model, version, created_at_epoch, updated_at_epoch)
         VALUES ('user:default', 'user', 'user:default', 'project', '/repo',
                 'bad sources', ?1, ?2, ?3, 'active', 'test', 1, 10, 10)",
        params![
            source_claim_ids_json,
            source_memory_ids_json,
            source_activity_refs_json
        ],
    )
}
