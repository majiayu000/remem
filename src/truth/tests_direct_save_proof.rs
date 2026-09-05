//! Counterexamples for treating save_memory receipts as G2 writer proof.
//!
//! Product (`docs/specs/legacy-unverified-context/PRODUCT.md`): only
//! evidence-backed rows are current. Writer-proof arms that skip generated
//! provenance are explicit `user_prompt` saves and proven lesson metadata.
//! `memory_claims` is a short-lived summary-dedup receipt (v026), not source
//! proof. Unknown proof fails closed. REST agent saves are `external_content`.
//!
//! All three groups below must be rejected.

use super::{classify_memory, MemoryVisibilityReason};
use anyhow::Result;
use rusqlite::Connection;

fn insert_active_bugfix(
    conn: &Connection,
    id: i64,
    content: &str,
    files: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, files, created_at_epoch,
          updated_at_epoch, status, source_trust_class)
         VALUES (?1, '/repo', 'saved decision', ?2, 'bugfix', ?3,
                 1, 1, 'active', 'external_content')",
        rusqlite::params![id, content, files],
    )?;
    Ok(())
}

fn insert_claim(conn: &Connection, memory_id: i64, fingerprint: &str, preview: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_claims
         (memory_id, project, claim_source, memory_type, content_fingerprint,
          content_preview, created_at_epoch)
         VALUES (?1, '/repo', 'case-save', 'bugfix', ?2, ?3, 1)",
        rusqlite::params![memory_id, fingerprint, preview],
    )?;
    Ok(())
}

fn assert_not_current(conn: &Connection, id: i64, expected: MemoryVisibilityReason) -> Result<()> {
    let visibility = classify_memory(conn, id, 2)?;
    assert!(
        !visibility.current_context_eligible,
        "id={id} must not be current, got {:?}",
        visibility.reason
    );
    assert_eq!(visibility.reason, expected);
    Ok(())
}

/// Group 1: arbitrary nonempty path + a save claim, no verifiable source.
#[test]
fn arbitrary_unverified_path_with_claim_is_not_current() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    insert_active_bugfix(&conn, 1, "agent body", Some(r#"["not-a-real-source"]"#))?;
    insert_claim(&conn, 1, "fingerprint", "preview")?;
    assert_not_current(&conn, 1, MemoryVisibilityReason::ProvenanceMissing)
}

/// Group 2: leftover claim for a previous body must not admit rewritten text.
#[test]
fn stale_claim_does_not_admit_rewritten_body() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    insert_active_bugfix(&conn, 1, "original body", Some(r#"["reports/source.md"]"#))?;
    insert_claim(&conn, 1, "fingerprint-of-original-body", "original body")?;
    conn.execute(
        "UPDATE memories SET content = 'rewritten body' WHERE id = 1",
        [],
    )?;
    assert_not_current(&conn, 1, MemoryVisibilityReason::ProvenanceMissing)
}

/// Group 3a: files + claim must not skip dangling evidence rejection.
#[test]
fn files_and_claim_do_not_bypass_invalid_evidence() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, files, evidence_event_ids,
          confidence, valid_from_epoch, created_at_epoch, updated_at_epoch,
          status, source_trust_class)
         VALUES (1, '/repo', 'saved decision', 'body', 'bugfix',
                 '[\"reports/source.md\"]', '[999]', 0.95, 1, 1, 1,
                 'active', 'external_content')",
        [],
    )?;
    insert_claim(&conn, 1, "fingerprint", "preview")?;
    assert_not_current(&conn, 1, MemoryVisibilityReason::ProvenanceMalformed)
}

/// Group 3b: files + claim must not skip the confidence floor.
#[test]
fn files_and_claim_do_not_bypass_confidence_floor() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    insert_active_bugfix(&conn, 1, "body", Some(r#"["reports/source.md"]"#))?;
    crate::truth::test_support::seed_current_memory_direct_evidence_proof(&conn, 1)?;
    conn.execute("UPDATE memories SET confidence = 0.5 WHERE id = 1", [])?;
    insert_claim(&conn, 1, "fingerprint", "preview")?;
    assert_not_current(&conn, 1, MemoryVisibilityReason::ConfidenceBelowFloor)
}
