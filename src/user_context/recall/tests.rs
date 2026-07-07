use anyhow::Result;
use rusqlite::Connection;

use super::*;
use crate::{
    memory,
    memory::suppression::{create_suppression, parse_target, SuppressRequest},
    user_context::claims::{
        create_manual_claim, suppress_claim, ManualClaimRequest, UserContextClaimType,
        UserContextSensitivity,
    },
};

fn migrated_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn request(query: &str) -> UserRecallRequest {
    UserRecallRequest {
        query: query.to_string(),
        project: "/repo".to_string(),
        task_intent: None,
        current_files: Vec::new(),
        host: None,
        owner_scope: None,
        owner_key: None,
        state_keys: Vec::new(),
        include_sensitive: false,
        include_suppressed: false,
        limit: Some(10),
        budget_chars: Some(4_000),
    }
}

fn claim(conn: &Connection, text: &str, sensitivity: UserContextSensitivity) -> Result<i64> {
    Ok(create_manual_claim(
        conn,
        &ManualClaimRequest {
            text,
            owner_scope: None,
            owner_key: None,
            claim_type: UserContextClaimType::Preference,
            claim_key: None,
            confidence: 1.0,
            sensitivity,
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?
    .id)
}

#[test]
fn recall_combines_user_claim_repo_memory_workstream_and_session() -> Result<()> {
    let conn = migrated_conn()?;
    let claim_id = claim(
        &conn,
        "Prefer concise recall architecture reviews for remem",
        UserContextSensitivity::Normal,
    )?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-1"),
        "/repo",
        Some("recall-design"),
        "Recall design",
        "The recall design should stay compact and source attributed.",
        "decision",
        None,
    )?;
    crate::workstream::upsert_workstream(
        &conn,
        "/repo",
        "ws-session",
        &crate::workstream::ParsedWorkStream {
            title: Some("Ship recall context overlay".to_string()),
            progress: Some("recall progress".to_string()),
            next_action: Some("wire MCP recall".to_string()),
            blockers: None,
            is_completed: false,
        },
    )?;
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at_epoch)
         VALUES ('s1', '/repo', 'recall context request', 'implemented recall tests', 10)",
        [],
    )?;

    let result = recall_user_context(&conn, &request("recall"))?;

    assert!(!result.empty);
    assert_eq!(
        result.usage_policy,
        Some(crate::user_context::usage_policy::USER_CONTEXT_USAGE_POLICY)
    );
    assert!(result.context.contains("recall"));
    assert!(!result
        .context
        .contains(crate::user_context::usage_policy::USER_CONTEXT_USAGE_POLICY));
    assert!(result
        .included
        .iter()
        .any(|item| item.source_type == "user_claim" && item.source_id == Some(claim_id)));
    assert!(result
        .included
        .iter()
        .any(|item| item.source_type == "memory" && item.source_id == Some(memory_id)));
    assert!(result
        .included
        .iter()
        .any(|item| item.source_type == "workstream"));
    assert!(result
        .included
        .iter()
        .any(|item| item.source_type == "session_summary"));
    Ok(())
}

#[test]
fn recall_includes_semantic_rollup_session_but_excludes_synthetic_range_title() -> Result<()> {
    let conn = migrated_conn()?;
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, decisions, learned,
          next_steps, preferences, created_at_epoch, session_row_id,
          covered_from_event_id, covered_to_event_id)
         VALUES
         ('rollup-1', '/repo', 'Captured event range 1..3', 'synthetic rollup text',
          '', '', '', '', 10, 1, 1, 3),
         ('rollup-2', '/repo', 'rollup structured recall', 'semantic rollup text',
          'SessionRollup owns structured fields.', 'reader migration works',
          'keep regression coverage', 'preserve preferences', 11, 2, 4, 6)",
        [],
    )?;

    let result = recall_user_context(&conn, &request("rollup structured"))?;

    assert!(!result.empty);
    assert!(result.included.iter().any(|item| {
        item.source_type == "session_summary"
            && item.title.as_deref() == Some("rollup structured recall")
            && item.text.contains("SessionRollup owns structured fields")
            && item.text.contains("preserve preferences")
    }));
    assert!(!result.context.contains("Captured event range 1..3"));
    Ok(())
}

#[test]
fn recall_returns_user_only_claim_context() -> Result<()> {
    let conn = migrated_conn()?;
    let claim_id = claim(
        &conn,
        "Prefer user-only recall examples for agent memory docs",
        UserContextSensitivity::Normal,
    )?;

    let result = recall_user_context(&conn, &request("user-only recall"))?;

    assert!(!result.empty);
    assert!(result.context.contains("user-only recall"));
    assert!(result
        .included
        .iter()
        .any(|item| item.source_type == "user_claim" && item.source_id == Some(claim_id)));
    assert!(!result
        .included
        .iter()
        .any(|item| item.source_type == "memory"));
    Ok(())
}

#[test]
fn recall_returns_repo_only_memory_context() -> Result<()> {
    let conn = migrated_conn()?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-1"),
        "/repo",
        Some("repo-only-recall"),
        "Repo-only recall",
        "Repo-only recall should work even when there are no user claims.",
        "decision",
        None,
    )?;

    let result = recall_user_context(&conn, &request("repo-only recall"))?;

    assert!(!result.empty);
    assert!(result
        .included
        .iter()
        .any(|item| item.source_type == "memory" && item.source_id == Some(memory_id)));
    assert!(!result
        .included
        .iter()
        .any(|item| item.source_type == "user_claim"));
    Ok(())
}

#[test]
fn recall_includes_explicit_current_state_key() -> Result<()> {
    let conn = migrated_conn()?;
    seed_current_state(&conn)?;
    let mut req = request("deploy");
    req.state_keys = vec!["deploy-target".to_string()];

    let result = recall_user_context(&conn, &req)?;

    assert!(!result.empty);
    assert!(result.included.iter().any(|item| {
        item.source_type == "current_state"
            && item.title.as_deref() == Some("current state: deploy-target")
            && item
                .reason_codes
                .contains(&"state_key:deploy-target".to_string())
    }));
    Ok(())
}

#[test]
fn recall_excludes_sensitive_expired_and_suppressed_by_default() -> Result<()> {
    let conn = migrated_conn()?;
    claim(
        &conn,
        "Sensitive recall identity detail",
        UserContextSensitivity::Sensitive,
    )?;
    create_manual_claim(
        &conn,
        &ManualClaimRequest {
            text: "Expired recall preference",
            owner_scope: None,
            owner_key: None,
            claim_type: UserContextClaimType::Preference,
            claim_key: None,
            confidence: 1.0,
            sensitivity: UserContextSensitivity::Normal,
            valid_from_epoch: None,
            valid_to_epoch: Some(1),
        },
    )?;
    let policy_suppressed = claim(
        &conn,
        "Policy suppressed recall preference",
        UserContextSensitivity::Normal,
    )?;
    let status_suppressed = claim(
        &conn,
        "Status suppressed recall preference",
        UserContextSensitivity::Normal,
    )?;
    suppress_claim(&conn, status_suppressed)?;
    let rejected = claim(
        &conn,
        "Rejected recall preference",
        UserContextSensitivity::Normal,
    )?;
    conn.execute(
        "UPDATE user_context_claims SET status = 'rejected' WHERE id = ?1",
        [rejected],
    )?;
    create_suppression(
        &conn,
        &SuppressRequest {
            target: parse_target(&format!("claim:{policy_suppressed}"))?,
            reason: Some("hide recall"),
            actor: Some("test"),
        },
    )?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-1"),
        "/repo",
        Some("recall-suppressed"),
        "Suppressed recall memory",
        "Suppressed recall memory body",
        "decision",
        None,
    )?;
    create_suppression(
        &conn,
        &SuppressRequest {
            target: parse_target(&format!("memory:{memory_id}"))?,
            reason: Some("hide memory"),
            actor: Some("test"),
        },
    )?;

    let result = recall_user_context(&conn, &request("recall"))?;

    assert!(result.included.is_empty());
    assert!(result.dropped.iter().any(|item| {
        item.source_type == "user_claim" && item.reason_code == "sensitivity:sensitive"
    }));
    assert!(result
        .dropped
        .iter()
        .any(|item| item.source_type == "user_claim" && item.reason_code == "expired"));
    assert!(result
        .dropped
        .iter()
        .any(|item| item.source_type == "user_claim" && item.reason_code == "policy_suppressed"));
    assert!(result.dropped.iter().any(|item| {
        item.source_type == "user_claim" && item.reason_code == "status:suppressed"
    }));
    assert!(result
        .dropped
        .iter()
        .any(|item| item.source_type == "user_claim" && item.reason_code == "status:rejected"));

    let mut audit_req = request("recall");
    audit_req.include_sensitive = true;
    audit_req.include_suppressed = true;
    let audit = recall_user_context(&conn, &audit_req)?;
    assert!(audit
        .included
        .iter()
        .any(|item| item.source_type == "user_claim" && item.source_id == Some(status_suppressed)));
    assert!(!audit
        .included
        .iter()
        .any(|item| item.source_type == "user_claim" && item.source_id == Some(rejected)));
    assert!(audit
        .dropped
        .iter()
        .any(|item| item.source_type == "user_claim"
            && item.source_id == Some(rejected)
            && item.reason_code == "status:rejected"));
    assert!(audit
        .included
        .iter()
        .any(|item| item.source_type == "memory" && item.source_id == Some(memory_id)));
    Ok(())
}

#[test]
fn recall_returns_explicit_empty_result_without_generic_profile() -> Result<()> {
    let conn = migrated_conn()?;
    claim(
        &conn,
        "Prefer compact Rust tests",
        UserContextSensitivity::Normal,
    )?;

    let result = recall_user_context(&conn, &request("nonexistent-topic"))?;

    assert!(result.empty);
    assert!(result.context.is_empty());
    assert!(result.usage_policy.is_none());
    assert!(result.included.is_empty());
    assert!(result
        .dropped
        .iter()
        .any(|item| item.reason_code == "not_relevant"));
    Ok(())
}

#[test]
fn recall_output_respects_budget() -> Result<()> {
    let conn = migrated_conn()?;
    for idx in 0..8 {
        claim(
            &conn,
            &format!("recall budget preference {idx} {}", "long ".repeat(120)),
            UserContextSensitivity::Normal,
        )?;
    }
    let mut req = request("recall budget");
    req.budget_chars = Some(500);
    let result = recall_user_context(&conn, &req)?;

    assert!(result.context.chars().count() <= 500);
    assert!(result
        .dropped
        .iter()
        .any(|item| item.reason_code == "budget_exceeded"));
    Ok(())
}

fn seed_current_state(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label,
          state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (10, 'repo', '/repo', 'decision', 'deploy-target',
                 'deploy target', 'active', NULL, 1700000000, 1700000010)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope, source_project,
          target_project, owner_scope, owner_key, context_class, state_key_id)
         VALUES (2, NULL, '/repo', 'deploy-target', 'Deploy target',
                 'Use production for deploy recall.', 'decision', NULL,
                 1700000002, 1700000002, 'active', NULL, 'project', '/repo',
                 '/repo', 'repo', '/repo', 'startup_core', 10)",
        [],
    )?;
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = 2 WHERE id = 10",
        [],
    )?;
    Ok(())
}
