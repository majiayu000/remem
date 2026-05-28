use anyhow::Result;
use rusqlite::{params, Connection};

use super::*;
use crate::memory::insert_memory;
use crate::memory::tests_helper::setup_memory_schema;

#[test]
fn govern_memories_requires_reason_and_confirmation_for_mutation() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let id = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        Some("bad-memory"),
        "Bad memory",
        "This memory should be rejected.",
        "discovery",
        None,
    )?;

    let no_confirm = govern_memories(
        &conn,
        &GovernMemoryRequest {
            project: "proj",
            ids: &[id],
            action: MemoryGovernanceAction::Reject,
            reason: Some("wrong"),
            actor: Some("test"),
            dry_run: false,
            confirm_destructive: false,
        },
    )
    .expect_err("mutation should require confirmation");
    assert!(no_confirm.to_string().contains("confirm_destructive"));

    let no_reason = govern_memories(
        &conn,
        &GovernMemoryRequest {
            project: "proj",
            ids: &[id],
            action: MemoryGovernanceAction::Reject,
            reason: None,
            actor: Some("test"),
            dry_run: false,
            confirm_destructive: true,
        },
    )
    .expect_err("mutation should require reason");
    assert!(no_reason.to_string().contains("explicit reason"));

    let status: String =
        conn.query_row("SELECT status FROM memories WHERE id = ?1", [id], |row| {
            row.get(0)
        })?;
    assert_eq!(status, "active");
    Ok(())
}

#[test]
fn govern_memories_dry_run_lists_targets_without_mutation_or_audit() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let id = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        Some("bad-memory"),
        "Bad memory",
        "This memory should be previewed.",
        "discovery",
        None,
    )?;

    let result = govern_memories(
        &conn,
        &GovernMemoryRequest {
            project: "proj",
            ids: &[id],
            action: MemoryGovernanceAction::Delete,
            reason: None,
            actor: Some("test"),
            dry_run: true,
            confirm_destructive: false,
        },
    )?;

    assert!(result.dry_run);
    assert_eq!(result.affected.len(), 1);
    assert_eq!(result.affected[0].previous_status, "active");
    assert_eq!(result.affected[0].new_status, "deleted");
    let status: String =
        conn.query_row("SELECT status FROM memories WHERE id = ?1", [id], |row| {
            row.get(0)
        })?;
    assert_eq!(status, "active");
    let audit_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = 'memory_governance'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(audit_count, 0);
    Ok(())
}

#[test]
fn govern_memories_writes_audit_and_removes_deleted_memory_from_fts() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let id = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        Some("bad-memory"),
        "Bad needle memory",
        "This needle should disappear from FTS after deletion.",
        "discovery",
        None,
    )?;
    assert_eq!(
        crate::memory::search_memories_fts(&conn, "needle", Some("proj"), None, 10, 0)?.len(),
        1
    );

    let result = govern_memories(
        &conn,
        &GovernMemoryRequest {
            project: "proj",
            ids: &[id],
            action: MemoryGovernanceAction::Delete,
            reason: Some("incorrect memory"),
            actor: Some("codex-test"),
            dry_run: false,
            confirm_destructive: true,
        },
    )?;

    assert_eq!(result.affected.len(), 1);
    assert_eq!(result.affected[0].new_status, "deleted");
    assert!(
        crate::memory::search_memories_fts(&conn, "needle", Some("proj"), None, 10, 0)?.is_empty()
    );
    let (summary, detail): (String, String) = conn.query_row(
        "SELECT summary, detail FROM events WHERE event_type = 'memory_governance'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert!(summary.contains("active -> deleted"));
    assert!(detail.contains("\"memory_id\":"));
    assert!(detail.contains("\"previous_status\":\"active\""));
    assert!(detail.contains("\"new_status\":\"deleted\""));
    assert!(detail.contains("incorrect memory"));
    Ok(())
}

#[test]
fn rejected_memories_stay_hidden_when_include_stale_is_true() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let rejected = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        Some("rejected-memory"),
        "Rejected memory",
        "Rejected governance content.",
        "discovery",
        None,
    )?;
    let archived = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        Some("archived-memory"),
        "Archived memory",
        "Archived governance content.",
        "discovery",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET status = 'archived' WHERE id = ?1",
        params![archived],
    )?;
    govern_memories(
        &conn,
        &GovernMemoryRequest {
            project: "proj",
            ids: &[rejected],
            action: MemoryGovernanceAction::Reject,
            reason: Some("bad extraction"),
            actor: Some("codex-test"),
            dry_run: false,
            confirm_destructive: true,
        },
    )?;

    let results = crate::memory::service::search_memories(
        &conn,
        &crate::memory::service::SearchRequest {
            project: Some("proj".to_string()),
            include_stale: true,
            limit: 10,
            ..crate::memory::service::SearchRequest::default()
        },
    )?;
    let titles: Vec<_> = results
        .memories
        .iter()
        .map(|memory| memory.title.as_str())
        .collect();

    assert_eq!(titles, vec!["Archived memory"]);
    Ok(())
}
