use super::*;
use rusqlite::params;

fn setup_gate_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../migrations/v016_context_injection_gate.sql"
    ))
    .unwrap();
    conn
}

fn gate_invocation(session_id: Option<&str>) -> ContextInvocation {
    ContextInvocation {
        cwd: "/tmp/remem".to_string(),
        project: "/tmp/remem".to_string(),
        session_id: session_id.map(str::to_string),
        transcript_path: Some("/tmp/remem.jsonl".to_string()),
        source: None,
        host: HostKind::CodexCli,
        use_colors: false,
        debug: false,
        force: false,
        gate_mode: None,
    }
}

#[test]
fn fingerprint_ignores_header_timestamp() {
    let a = "# [/tmp/remem] context 2026-05-25 1:00pm\nBody\n";
    let b = "# [/tmp/remem] context 2026-05-25 2:00pm\nBody\n";

    assert_eq!(context_fingerprint(a), context_fingerprint(b));
}

#[test]
fn fingerprint_ignores_footer_totals_derived_from_header_width() {
    let a = "# [/tmp/remem] context 2026-05-25 9:00am\nBody\n\n1 context memories loaded. 1 core (10 chars). 0 lessons (0 chars). 0 indexed (0 chars). 0 preferences (project:0 global:0, 0 chars). 0 sessions (0 chars). host=codex-cli branch=main total=100 chars/~25 tokens limit=12000 truncated=no\n";
    let b = "# [/tmp/remem] context 2026-05-25 10:00am\nBody\n\n1 context memories loaded. 1 core (10 chars). 0 lessons (0 chars). 0 indexed (0 chars). 0 preferences (project:0 global:0, 0 chars). 0 sessions (0 chars). host=codex-cli branch=main total=101 chars/~26 tokens limit=12000 truncated=no\n";

    assert_eq!(context_fingerprint(a), context_fingerprint(b));
}

#[test]
fn fingerprint_ignores_context_source_note() {
    let normal =
        "# [/tmp/remem] context now\nUse `search`/`get_observations` for details.\n\nBody\n";
    let post_compact = "# [/tmp/remem] context now [REMEM POST-COMPACT RELOAD]\nUse `search`/`get_observations` for details.\nREMEM_CONTEXT_SOURCE=compact: Codex compact triggered this memory context reload.\n\nBody\n";
    let current_compact = "# [/tmp/remem] context now\nUse `search`/`get_observations` for details.\n\nCodex compacted the chat, so remem refreshed memory context.\n\nBody\n";
    let current_clear = "# [/tmp/remem] context now\nUse `search`/`get_observations` for details.\n\nContext was reloaded after an explicit clear.\n\nBody\n";

    assert_eq!(
        context_fingerprint(normal),
        context_fingerprint(post_compact)
    );
    assert_eq!(
        context_fingerprint(normal),
        context_fingerprint(current_compact)
    );
    assert_eq!(
        context_fingerprint(normal),
        context_fingerprint(current_clear)
    );
}

#[test]
fn fingerprint_ignores_indented_terminal_header_source_and_timestamp() {
    let a = "remem context\n              ├─ project: /tmp/remem\n              ├─ source: compact\n              └─ updated: 2026-06-02 6:44pm +08:00\nBody\n";
    let b = "remem context\n              ├─ project: /tmp/remem\n              ├─ source: compact\n              └─ updated: 2026-06-02 6:45pm +08:00\nBody\n";

    assert_eq!(context_fingerprint(a), context_fingerprint(b));
}

#[test]
fn fingerprint_ignores_codex_color_indent_and_visual_footer_totals() {
    let plain = "remem context\n├─ project: /tmp/remem\n├─ branch: main\n└─ updated: 2026-06-02 6:44pm +08:00\nBody\n\nLoaded\n├─ Memories: 1 total, 1 core, 0 lessons, 0 indexed\n├─ Preferences: 0 total, 0 project, 0 global\n├─ Sessions: 0\n├─ Workstreams: 0\n└─ Budget: 100 chars (~25 tokens) / 12000, truncated: no\n";
    let colored = "\x1b[1;36mremem context\x1b[0m\n              ├─ \x1b[1mproject\x1b[0m: /tmp/remem\n              ├─ \x1b[1mbranch\x1b[0m: main\n              └─ \x1b[1mupdated\x1b[0m: 2026-06-02 6:45pm +08:00\nBody\n\n\x1b[1;36mLoaded\x1b[0m\n├─ \x1b[1mMemories\x1b[0m: 1 total, 1 core, 0 lessons, 0 indexed\n├─ \x1b[1mPreferences\x1b[0m: 0 total, 0 project, 0 global\n├─ \x1b[1mSessions\x1b[0m: 0\n├─ \x1b[1mWorkstreams\x1b[0m: 0\n└─ \x1b[1mBudget\x1b[0m: 101 chars (~26 tokens) / 12000, truncated: no\n";

    assert_eq!(context_fingerprint(plain), context_fingerprint(colored));
}

#[test]
fn fingerprint_keeps_non_footer_total_text() {
    let a = "# [/tmp/remem] context now\nBody total=100 chars/~25 tokens\n";
    let b = "# [/tmp/remem] context now\nBody total=101 chars/~26 tokens\n";

    assert_ne!(context_fingerprint(a), context_fingerprint(b));
}

#[test]
fn first_same_session_context_emits_and_second_suppresses() -> Result<()> {
    let conn = setup_gate_conn();
    let invocation = gate_invocation(Some("sess-1"));
    let key = injection_key(&invocation);
    let hash = context_fingerprint("# [/tmp/remem] context now\nBody\n");

    assert!(load_gate_row(&conn, invocation.host.as_env_value(), &key)?.is_none());
    upsert_emit_row(&conn, &invocation, &key, &hash, "full", 32, 100)?;
    let row = load_gate_row(&conn, invocation.host.as_env_value(), &key)?
        .ok_or_else(|| anyhow::anyhow!("missing context injection gate row"))?;
    assert_eq!(row.context_hash, hash);

    record_suppression(&conn, &invocation, &key, 101)?;
    let suppress_count: i64 = conn.query_row(
        "SELECT suppress_count FROM context_injections WHERE host = ?1 AND injection_key = ?2",
        params![invocation.host.as_env_value(), key],
        |row| row.get(0),
    )?;
    assert_eq!(suppress_count, 1);
    Ok(())
}

#[test]
fn fallback_injection_key_canonicalizes_equivalent_cwd() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut direct = gate_invocation(None);
    direct.transcript_path = None;
    direct.cwd = cwd.to_string_lossy().to_string();

    let mut dotted = direct.clone();
    dotted.cwd = cwd.join(".").to_string_lossy().to_string();

    assert_eq!(injection_key(&direct), injection_key(&dotted));
    Ok(())
}

#[test]
fn transcript_fallback_injection_key_canonicalizes_equivalent_cwd() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut direct = gate_invocation(None);
    direct.cwd = cwd.to_string_lossy().to_string();
    direct.transcript_path = Some("/tmp/remem-transcript.jsonl".to_string());

    let mut dotted = direct.clone();
    dotted.cwd = cwd.join(".").to_string_lossy().to_string();

    assert_eq!(injection_key(&direct), injection_key(&dotted));
    Ok(())
}

#[test]
fn cwd_only_fallback_identity_fails_open_without_suppressing() {
    let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-cwd-only");
    let mut invocation = gate_invocation(None);
    invocation.transcript_path = None;
    let output = "# [/tmp/remem] context now\nBody\n".to_string();

    let first = apply_context_gate(&invocation, output.clone());
    assert_eq!(first.action, ContextGateAction::FailOpen);
    assert_eq!(first.reason, "missing_session_identity");
    assert_eq!(first.output, output);

    let second = apply_context_gate(&invocation, output.clone());
    assert_eq!(second.action, ContextGateAction::FailOpen);
    assert_eq!(second.reason, "missing_session_identity");
    assert_eq!(second.output, output);
}

#[test]
fn suppression_does_not_extend_fallback_cooldown() -> Result<()> {
    let conn = setup_gate_conn();
    let invocation = gate_invocation(None);
    let key = injection_key(&invocation);
    let hash = context_fingerprint("# [/tmp/remem] context now\nBody\n");

    upsert_emit_row(&conn, &invocation, &key, &hash, "full", 32, 100)?;
    record_suppression(&conn, &invocation, &key, 150)?;

    let (updated_at_epoch, last_emitted_epoch, suppress_count): (i64, i64, i64) = conn.query_row(
        "SELECT updated_at_epoch, last_emitted_epoch, suppress_count
             FROM context_injections
             WHERE host = ?1 AND injection_key = ?2",
        params![invocation.host.as_env_value(), key],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;

    assert_eq!(updated_at_epoch, 150);
    assert_eq!(last_emitted_epoch, 100);
    assert_eq!(suppress_count, 1);
    Ok(())
}

#[test]
fn restart_source_requires_fresh_emission() {
    assert!(source_requires_fresh_emission(Some("clear")));
    assert!(!source_requires_fresh_emission(Some("Compact")));
    assert!(!source_requires_fresh_emission(Some("startup")));
    assert!(!source_requires_fresh_emission(None));
}

#[test]
fn compact_source_is_suppressed_by_default() {
    assert!(source_is_suppressed(Some("Compact")));
    assert!(!source_is_suppressed(Some("clear")));
    assert!(!source_is_suppressed(Some("startup")));
    assert!(!source_is_suppressed(None));
}

#[test]
fn compact_source_emits_first_context() {
    let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-compact-first");
    let mut invocation = gate_invocation(Some("sess-compact-first"));
    invocation.source = Some("compact".to_string());
    let output = "# [/tmp/remem] context now\nBody\n".to_string();

    let decision = apply_context_gate(&invocation, output.clone());
    assert_eq!(decision.action, ContextGateAction::EmittedFull);
    assert_eq!(decision.reason, "first_or_forced");
    assert_eq!(decision.output, output);
    assert_eq!(decision.output_mode, Some("full"));
    assert!(decision
        .key
        .as_deref()
        .is_some_and(|key| key.contains("sess-compact-first")));
    assert!(decision.context_hash.is_some());
}

#[test]
fn compact_source_suppresses_same_hash_after_first_context() {
    let _data_dir =
        crate::db::test_support::ScopedTestDataDir::new("context-gate-compact-same-hash");
    let mut invocation = gate_invocation(Some("sess-compact-same-hash"));
    let output = "# [/tmp/remem] context now\nBody\n".to_string();

    let first = apply_context_gate(&invocation, output.clone());
    assert_eq!(first.action, ContextGateAction::EmittedFull);

    invocation.source = Some("compact".to_string());
    let second = apply_context_gate(&invocation, output);
    assert_eq!(second.action, ContextGateAction::Suppressed);
    assert_eq!(second.reason, "suppressed_source");
    assert!(second.output.is_empty());
}

#[test]
fn compact_source_force_bypasses_suppression() {
    let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-compact-force");
    let mut invocation = gate_invocation(Some("sess-compact-force"));
    let output = "# [/tmp/remem] context now\nBody\n".to_string();

    let first = apply_context_gate(&invocation, output.clone());
    assert_eq!(first.action, ContextGateAction::EmittedFull);

    invocation.source = Some("compact".to_string());
    invocation.force = true;
    let forced = apply_context_gate(&invocation, output.clone());
    assert_eq!(forced.action, ContextGateAction::EmittedFull);
    assert_eq!(forced.reason, "first_or_forced");
    assert_eq!(forced.output, output);
}

#[test]
fn compact_source_missing_identity_fails_open() {
    let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-compact-no-id");
    let mut invocation = gate_invocation(None);
    invocation.transcript_path = None;
    invocation.source = Some("compact".to_string());
    let output = "# [/tmp/remem] context now\nBody\n".to_string();

    let decision = apply_context_gate(&invocation, output.clone());
    assert_eq!(decision.action, ContextGateAction::FailOpen);
    assert_eq!(decision.reason, "missing_session_identity");
    assert_eq!(decision.output, output);
}

#[test]
fn compact_source_changed_hash_emits_delta() {
    let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-compact-changed");
    let mut invocation = gate_invocation(Some("sess-compact-changed"));

    let first = apply_context_gate(
        &invocation,
        "# [/tmp/remem] context now\nBody A\n".to_string(),
    );
    assert_eq!(first.action, ContextGateAction::EmittedFull);

    invocation.source = Some("compact".to_string());
    let second = apply_context_gate(
        &invocation,
        "# [/tmp/remem] context now\nBody B\n".to_string(),
    );
    assert_eq!(second.action, ContextGateAction::EmittedDelta);
    assert_eq!(second.reason, "changed_hash");
    assert!(second.output.contains("context delta"));
}

#[test]
fn strict_mode_suppresses_changed_hash_after_first_context() {
    let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-strict-changed");
    let mut invocation = gate_invocation(Some("sess-strict-changed"));
    invocation.gate_mode = Some("strict".to_string());

    let first = apply_context_gate(
        &invocation,
        "# [/tmp/remem] context now\nBody A\n".to_string(),
    );
    assert_eq!(first.action, ContextGateAction::EmittedFull);

    let second = apply_context_gate(
        &invocation,
        "# [/tmp/remem] context now\nBody B\n".to_string(),
    );
    assert_eq!(second.action, ContextGateAction::Suppressed);
    assert_eq!(second.reason, "strict");
    assert!(second.output.is_empty());
}

#[test]
fn restart_source_reemits_same_hash_context() {
    let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-restart");
    let mut invocation = gate_invocation(Some("sess-restart"));
    let output = "# [/tmp/remem] context now\nBody\n".to_string();

    let first = apply_context_gate(&invocation, output.clone());
    assert_eq!(first.action, ContextGateAction::EmittedFull);

    let second = apply_context_gate(&invocation, output.clone());
    assert_eq!(second.action, ContextGateAction::Suppressed);

    invocation.source = Some("clear".to_string());
    let restart = apply_context_gate(&invocation, output.clone());
    assert_eq!(restart.action, ContextGateAction::EmittedFull);
    assert_eq!(restart.reason, "restart_source");
    assert_eq!(restart.output, output);
}

#[test]
fn emit_and_suppress_persist_hook_source() -> Result<()> {
    let conn = setup_gate_conn();
    let mut invocation = gate_invocation(Some("sess-source"));
    invocation.source = Some("startup".to_string());
    let key = injection_key(&invocation);
    let hash = context_fingerprint("# [/tmp/remem] context now\nBody\n");

    upsert_emit_row(&conn, &invocation, &key, &hash, "full", 32, 100)?;
    invocation.source = Some("resume".to_string());
    record_suppression(&conn, &invocation, &key, 101)?;

    let hook_source: Option<String> = conn.query_row(
        "SELECT hook_source FROM context_injections WHERE host = ?1 AND injection_key = ?2",
        params![invocation.host.as_env_value(), key],
        |row| row.get(0),
    )?;

    assert_eq!(hook_source.as_deref(), Some("resume"));
    Ok(())
}

#[test]
fn fingerprint_keeps_user_debug_trace_heading() {
    let a = "# [/tmp/remem] context now\nBody\n## Debug Trace\nUser note A\n";
    let b = "# [/tmp/remem] context now\nBody\n## Debug Trace\nUser note B\n";

    assert_ne!(context_fingerprint(a), context_fingerprint(b));
}

#[test]
fn fingerprint_ignores_generated_debug_trace() {
    let base = "# [/tmp/remem] context now\nBody\n";
    let a = format!(
        "{}\n## Debug Trace\n- request host=codex-cli project=/tmp/remem cwd=/tmp/remem branch=main session=sess-1\n\n1 context memories loaded. 1 core (10 chars). 0 lessons (0 chars). 0 indexed (0 chars). 0 preferences (project:0 global:0, 0 chars). 0 sessions (0 chars). host=codex-cli branch=main total=100 chars/~25 tokens limit=12000 truncated=no\n",
        base
    );
    let b = format!(
        "{}\n## Debug Trace\n- request host=codex-cli project=/tmp/remem cwd=/tmp/remem branch=dev session=sess-2\n\n1 context memories loaded. 1 core (10 chars). 0 lessons (0 chars). 0 indexed (0 chars). 0 preferences (project:0 global:0, 0 chars). 0 sessions (0 chars). host=codex-cli branch=dev total=120 chars/~30 tokens limit=12000 truncated=no\n",
        base
    );

    assert_eq!(context_fingerprint(&a), context_fingerprint(&b));
}
