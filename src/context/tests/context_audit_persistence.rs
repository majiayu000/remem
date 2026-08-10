use super::super::audit::{
    finalize_items_for_decision, record_context_injection, record_context_injection_at,
    ContextAuditItem,
};
use super::super::host::HostKind;
use super::super::injection_gate::{ContextGateAction, ContextGateDecision};
use super::super::invocation::ContextInvocation;
use super::super::render::generate_context_for_test;
use super::insert_memory;
use crate::context_bundle::{
    ContextAudit, ContextBundle, DegradedMode, CONTEXT_BUNDLE_SCHEMA_VERSION,
};

fn injected_item(title: &str) -> ContextAuditItem {
    ContextAuditItem {
        item_kind: "memory",
        item_id: Some(42),
        memory_id: Some(42),
        channel: "index",
        score: Some(1.0),
        render_order: Some(1),
        status: "injected",
        drop_reason: None,
        title: title.to_string(),
        provenance: "src=memory:#42".to_string(),
        staleness: "fresh".to_string(),
        render_end_chars: Some(200),
    }
}

fn decision(action: ContextGateAction, output: &str) -> ContextGateDecision {
    ContextGateDecision {
        output: output.to_string(),
        action,
        reason: "test",
        key: None,
        context_hash: None,
        output_mode: None,
        retained_context_chars: (action == ContextGateAction::EmittedDelta).then_some(0),
    }
}

#[test]
fn full_gate_trusts_identity_safe_render_survivors_not_titles() {
    let title = "a very long title whose rendered form was truncated";
    let finalized = finalize_items_for_decision(
        &decision(ContextGateAction::EmittedFull, "#42 a very long title..."),
        &[injected_item(title)],
    );

    assert_eq!(finalized[0].status, "injected");
    assert_eq!(finalized[0].drop_reason, None);
}

#[test]
fn suppressed_and_delta_outputs_have_closed_drop_reasons() {
    let suppressed = finalize_items_for_decision(
        &decision(ContextGateAction::Suppressed, ""),
        &[injected_item("duplicate title")],
    );
    let delta = finalize_items_for_decision(
        &decision(ContextGateAction::EmittedDelta, "duplicate title"),
        &[injected_item("duplicate title")],
    );

    assert_eq!(suppressed[0].drop_reason, Some("gate_suppressed"));
    assert_eq!(delta[0].drop_reason, Some("delta_preview"));
}

#[test]
fn delta_keeps_items_with_identity_boundaries_inside_preview() {
    let mut delta_decision = decision(ContextGateAction::EmittedDelta, "preview");
    delta_decision.retained_context_chars = Some(250);

    let finalized = finalize_items_for_decision(&delta_decision, &[injected_item("title")]);

    assert_eq!(finalized[0].status, "injected");
    assert_eq!(finalized[0].drop_reason, None);
}

#[test]
fn bundle_audit_failure_rolls_back_item_rows_atomically() -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let plan_hash = "a".repeat(64);
    let invalid_bundle = ContextBundle {
        schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
        plan_hash: plan_hash.clone(),
        degraded_mode: DegradedMode::Full,
        preferences: Vec::new(),
        failure_lessons: Vec::new(),
        current_truth: Vec::new(),
        workstreams: Vec::new(),
        memory_index: Vec::new(),
        recent_sessions: Vec::new(),
        audit: ContextAudit {
            schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
            policy_version: "retrieval_router_v2".to_string(),
            relevance_policy_version: "sessionstart_significant_token_v1".to_string(),
            plan_hash,
            degraded_mode: DegradedMode::Full,
            candidates_considered: 0,
            selected_count: 0,
            dropped_count: 0,
            token_estimate: 0,
            token_budget: 0,
            truncation_reason: None,
            entries: Vec::new(),
        },
    };
    let invocation = ContextInvocation {
        cwd: "/repo".to_string(),
        project: "/repo".to_string(),
        session_id: Some("atomic-audit".to_string()),
        transcript_path: None,
        source: Some("startup".to_string()),
        host: HostKind::CodexCli,
        use_colors: false,
        debug: false,
        force: true,
        gate_mode: Some("off".to_string()),
    };

    let error = record_context_injection(
        &conn,
        &invocation,
        &decision(ContextGateAction::EmittedFull, "rendered payload"),
        &[injected_item("memory title")],
        Some(&invalid_bundle),
    )
    .expect_err("invalid bundle audit must fail the atomic write");
    let diagnostic = format!("{error:#}");
    assert!(diagnostic.contains("injection_run_id="), "{diagnostic}");
    assert!(
        diagnostic.contains("CHECK constraint failed: token_budget > 0"),
        "{diagnostic}"
    );

    let item_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM context_injection_items", [], |row| {
            row.get(0)
        })?;
    let audit_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM context_bundle_audits", [], |row| {
            row.get(0)
        })?;
    assert_eq!((item_count, audit_count), (0, 0));
    Ok(())
}

#[test]
fn context_audit_rows_reconstruct_injected_memories_for_session() -> anyhow::Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("context-audit-injected");
    let conn = crate::db::test_support::runtime_connection()?;
    insert_memory(
        &conn,
        1,
        data_dir.path.to_string_lossy().as_ref(),
        Some("audit-memory"),
        "decision",
        "Audit decision",
        "Audit body",
        chrono::Utc::now().timestamp(),
    );
    drop(conn);

    generate_context_for_test(
        ContextInvocation {
            cwd: data_dir.path.to_string_lossy().to_string(),
            project: data_dir.path.to_string_lossy().to_string(),
            session_id: Some("sess-audit-injected".to_string()),
            transcript_path: None,
            source: Some("session_start".to_string()),
            host: HostKind::CodexCli,
            use_colors: false,
            debug: false,
            force: true,
            gate_mode: None,
        },
        true,
    )?;

    let conn = crate::db::test_support::runtime_connection()?;
    let row: (i64, String, String, String) = conn.query_row(
        "SELECT memory_id, status, channel, provenance
         FROM context_injection_items
         WHERE session_id = 'sess-audit-injected' AND status = 'injected'
         ORDER BY render_order LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;

    assert_eq!(row.0, 1);
    assert_eq!(row.1, "injected");
    assert!(matches!(row.2.as_str(), "core" | "index"));
    assert!(row.3.contains("src=memory:#1"));

    let injection_run_id: String = conn.query_row(
        "SELECT injection_run_id FROM context_injection_items
         WHERE session_id = 'sess-audit-injected' LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let persisted = crate::context_bundle::persistence::load_verified_context_bundle_audit(
        &conn,
        &injection_run_id,
    )?
    .ok_or_else(|| anyhow::anyhow!("missing SessionStart Context Bundle audit"))?;
    assert_eq!(persisted.injection_run_id, injection_run_id);
    assert_eq!(persisted.audit.plan_hash.len(), 64);
    assert_eq!(
        persisted.audit.selected_count + persisted.audit.dropped_count,
        persisted.audit.candidates_considered
    );
    let audit_json: String = conn.query_row(
        "SELECT audit_json FROM context_bundle_audits WHERE injection_run_id = ?1",
        [&injection_run_id],
        |row| row.get(0),
    )?;
    assert!(!audit_json.contains("Audit decision"));
    assert!(!audit_json.contains("Audit body"));
    Ok(())
}

#[test]
fn empty_sessionstart_still_persists_bundle_contract() -> anyhow::Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("context-audit-empty");
    let project = data_dir.path.to_string_lossy().to_string();
    drop(crate::db::test_support::runtime_connection()?);

    generate_context_for_test(
        ContextInvocation {
            cwd: project.clone(),
            project,
            session_id: Some("sess-audit-empty".to_string()),
            transcript_path: None,
            source: Some("session_start".to_string()),
            host: HostKind::CodexCli,
            use_colors: false,
            debug: false,
            force: true,
            gate_mode: None,
        },
        true,
    )?;

    let conn = crate::db::test_support::runtime_connection()?;
    let row: (i64, i64, i64) = conn.query_row(
        "SELECT a.candidates_considered, a.selected_count, a.dropped_count
         FROM context_bundle_audits a
         JOIN context_injection_items i ON i.injection_run_id = a.injection_run_id
         WHERE i.session_id = 'sess-audit-empty'
         LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(row, (0, 0, 0));
    Ok(())
}

#[test]
fn distinct_same_second_invocations_keep_both_item_sets() -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let invocation = ContextInvocation {
        cwd: "/repo".to_string(),
        project: "/repo".to_string(),
        session_id: Some("same-second".to_string()),
        transcript_path: None,
        source: Some("UserPromptSubmit".to_string()),
        host: HostKind::ClaudeCode,
        use_colors: false,
        debug: false,
        force: false,
        gate_mode: Some("off".to_string()),
    };
    let decision = ContextGateDecision {
        output: String::new(),
        action: ContextGateAction::Bypassed,
        reason: "prompt_submit",
        key: None,
        context_hash: None,
        output_mode: Some("bypassed"),
        retained_context_chars: None,
    };
    let audit_item = |title: &str| ContextAuditItem {
        item_kind: "memory",
        item_id: Some(42),
        memory_id: Some(42),
        channel: "prompt_submit",
        score: Some(1.0),
        render_order: Some(1),
        status: "injected",
        drop_reason: None,
        title: title.to_string(),
        provenance: "src=memory:#42".to_string(),
        staleness: "fresh".to_string(),
        render_end_chars: None,
    };

    let first_run = record_context_injection_at(
        &conn,
        &invocation,
        &decision,
        &[audit_item("first prompt")],
        None,
        100,
    )?;
    let second_run = record_context_injection_at(
        &conn,
        &invocation,
        &decision,
        &[audit_item("second prompt")],
        None,
        100,
    )?;

    assert_ne!(first_run, second_run);
    let rows: (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COUNT(DISTINCT injection_run_id)
         FROM context_injection_items",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(rows, (2, 2));
    Ok(())
}
