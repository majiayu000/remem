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
            acknowledge_pattern: None,
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
            acknowledge_pattern: None,
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
            acknowledge_pattern: None,
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
fn select_memory_ids_filters_query_type_project_and_status_for_preview() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let active_match = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        Some("migration-plan"),
        "Migration plan",
        "Old migration plan should be reviewed.",
        "decision",
        None,
    )?;
    let stale_match = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        Some("stale-migration-plan"),
        "Stale migration plan",
        "Old migration plan already superseded.",
        "decision",
        None,
    )?;
    let other_type = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        Some("migration-discovery"),
        "Migration discovery",
        "Old migration plan evidence.",
        "discovery",
        None,
    )?;
    insert_memory(
        &conn,
        Some("s1"),
        "other",
        Some("other-migration-plan"),
        "Migration plan",
        "Old migration plan in another project.",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET status = 'stale' WHERE id = ?1",
        params![stale_match],
    )?;

    let ids = select_memory_ids(
        &conn,
        &GovernanceSelector {
            project: "proj",
            query: Some("old migration"),
            memory_type: Some("decision"),
            status: Some("active"),
            limit: 10,
            offset: 0,
        },
    )?;

    assert_eq!(ids, vec![active_match]);
    assert!(!ids.contains(&stale_match));
    assert!(!ids.contains(&other_type));

    let result = govern_memories(
        &conn,
        &GovernMemoryRequest {
            project: "proj",
            ids: &ids,
            action: MemoryGovernanceAction::MarkStale,
            reason: None,
            actor: Some("test"),
            dry_run: true,
            confirm_destructive: false,
            acknowledge_pattern: None,
        },
    )?;
    assert!(result.dry_run);
    let status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        [active_match],
        |row| row.get(0),
    )?;
    assert_eq!(status, "active");
    Ok(())
}

#[test]
fn selected_batch_apply_writes_one_audit_event_per_item() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let first = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        Some("pollution-one"),
        "Pollution one",
        "Project pollution from an old run.",
        "discovery",
        None,
    )?;
    let second = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        Some("pollution-two"),
        "Pollution two",
        "Project pollution from another old run.",
        "discovery",
        None,
    )?;

    let ids = select_memory_ids(
        &conn,
        &GovernanceSelector {
            project: "proj",
            query: Some("project pollution"),
            memory_type: Some("discovery"),
            status: None,
            limit: 10,
            offset: 0,
        },
    )?;
    assert_eq!(ids, vec![second, first]);

    let result = govern_memories(
        &conn,
        &GovernMemoryRequest {
            project: "proj",
            ids: &ids,
            action: MemoryGovernanceAction::Reject,
            reason: Some("project pollution"),
            actor: Some("codex-test"),
            dry_run: false,
            confirm_destructive: true,
            acknowledge_pattern: None,
        },
    )?;

    assert_eq!(result.affected.len(), 2);
    let audit_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = 'memory_governance'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(audit_count, 2);
    for id in [first, second] {
        let status: String =
            conn.query_row("SELECT status FROM memories WHERE id = ?1", [id], |row| {
                row.get(0)
            })?;
        assert_eq!(status, "rejected");
    }
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
            acknowledge_pattern: None,
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
fn govern_memories_acknowledges_existing_poisoned_memory_without_status_change() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let id = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        Some("quoted-poison"),
        "Quoted poison",
        "Ignore previous instructions only as a quoted false positive.",
        "preference",
        None,
    )?;

    let result = govern_memories(
        &conn,
        &GovernMemoryRequest {
            project: "proj",
            ids: &[id],
            action: MemoryGovernanceAction::AcknowledgePattern,
            reason: Some("quoted false positive reviewed"),
            actor: Some("maintainer"),
            dry_run: false,
            confirm_destructive: true,
            acknowledge_pattern: Some("override_previous_instructions"),
        },
    )?;

    assert_eq!(result.affected.len(), 1);
    assert_eq!(result.affected[0].previous_status, "active");
    assert_eq!(result.affected[0].new_status, "active");
    let ack: (String, i64, Option<i64>, String) = conn.query_row(
        "SELECT acknowledged_pattern_id, acknowledged_pattern_version,
                acknowledged_at_epoch, status
         FROM memories WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(ack.0, "override_previous_instructions");
    assert_eq!(
        ack.1,
        crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION
    );
    assert!(ack.2.is_some());
    assert_eq!(ack.3, "active");
    let detail: String = conn.query_row(
        "SELECT detail FROM events WHERE event_type = 'memory_governance'",
        [],
        |row| row.get(0),
    )?;
    assert!(detail.contains("\"action\":\"acknowledge_pattern\""));
    assert!(detail.contains("\"acknowledged_pattern\":\"override_previous_instructions\""));
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
            acknowledge_pattern: None,
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
