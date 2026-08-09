use super::super::host::HostKind;
use super::super::invocation::ContextInvocation;
use super::super::render::generate_context_for_test;
use super::insert_memory;

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
