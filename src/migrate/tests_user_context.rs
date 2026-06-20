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

#[test]
fn memory_suppression_and_feedback_migration_creates_rows_and_indexes() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    for name in [
        "memory_suppressions",
        "memory_feedback",
        "idx_memory_suppressions_target_active",
        "idx_memory_suppressions_owner_active",
        "idx_memory_feedback_target_recent",
        "idx_memory_feedback_context_item",
    ] {
        let exists: bool = conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE name = ?1",
            [name],
            |_| Ok(true),
        )?;
        assert!(exists, "{name} should exist after suppression migration");
    }

    conn.execute(
        "INSERT INTO memory_suppressions
         (target_kind, target_id, reason, actor, status, created_at_epoch, updated_at_epoch)
         VALUES ('memory', 1, 'not useful', 'cli', 'active', 10, 10)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_feedback
         (target_kind, target_id, feedback, source, created_at_epoch)
         VALUES ('memory', 1, 'not_relevant', 'cli', 10)",
        [],
    )?;
    Ok(())
}

#[test]
fn memory_suppression_and_feedback_targets_are_schema_validated() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    let err = conn
        .execute(
            "INSERT INTO memory_suppressions
             (target_kind, reason, actor, status, created_at_epoch, updated_at_epoch)
             VALUES ('memory', 'missing target', 'cli', 'active', 10, 10)",
            [],
        )
        .expect_err("suppression target id/value should be required");
    assert!(err.to_string().contains("CHECK"));

    let err = conn
        .execute(
            "INSERT INTO memory_feedback
             (target_kind, target_value, feedback, source, created_at_epoch)
             VALUES ('topic_key', 'abc', 'not-relevant', 'cli', 10)",
            [],
        )
        .expect_err("feedback values should use stable underscore ids");
    assert!(err.to_string().contains("CHECK"));
    Ok(())
}

#[test]
fn user_context_candidate_migration_creates_rows_and_indexes() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    for name in [
        "user_context_candidates",
        "idx_user_context_candidates_inbox",
        "idx_user_context_candidates_user_recent",
        "idx_user_context_candidates_dedupe",
    ] {
        let exists: bool = conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE name = ?1",
            [name],
            |_| Ok(true),
        )?;
        assert!(exists, "{name} should exist after candidate migration");
    }
    insert_candidate_row(
        &conn,
        r#"[{"kind":"session_summary","id":1}]"#,
        "pending_review",
        "low",
        "normal",
    )?;
    Ok(())
}

#[test]
fn user_context_candidate_schema_validates_sources_status_and_risk() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    let err = insert_candidate_row(&conn, "not-json", "pending_review", "low", "normal")
        .expect_err("source refs must be valid JSON");
    assert!(err.to_string().contains("CHECK"));
    let err = insert_candidate_row(&conn, "{}", "pending_review", "low", "normal")
        .expect_err("source refs must be an array");
    assert!(err.to_string().contains("CHECK"));
    let err = insert_candidate_row(&conn, "[]", "pending_review", "low", "normal")
        .expect_err("source refs must be non-empty");
    assert!(err.to_string().contains("CHECK"));
    let refs = r#"[{"kind":"session_summary","id":1}]"#;
    let err = insert_candidate_row(&conn, refs, "queued", "low", "normal")
        .expect_err("review status should be constrained");
    assert!(err.to_string().contains("CHECK"));
    let err = insert_candidate_row(&conn, refs, "pending_review", "unsafe", "normal")
        .expect_err("risk class should be constrained");
    assert!(err.to_string().contains("CHECK"));
    let err = insert_candidate_row(&conn, refs, "pending_review", "low", "private")
        .expect_err("sensitivity should be constrained");
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

fn insert_candidate_row(
    conn: &Connection,
    source_refs_json: &str,
    review_status: &str,
    risk_class: &str,
    sensitivity: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO user_context_candidates
         (user_key, owner_scope, owner_key, source_project, host, session_id,
          claim_type, claim_key, claim_text, confidence, sensitivity, risk_class,
          source_kind, source_refs_json, source_preview, review_status,
          auto_promote_block_reason, review_note, created_at_epoch, updated_at_epoch)
         VALUES ('user:default', 'user', 'user:default', '/repo', 'codex-cli', 's1',
                 'preference', 'preference:test', 'Prefer review-gated user context.',
                 0.9, ?4, ?3, 'session_summary', ?1, 'source preview', ?2,
                 'requires_review', NULL, 10, 10)",
        params![source_refs_json, review_status, risk_class, sensitivity],
    )
}
