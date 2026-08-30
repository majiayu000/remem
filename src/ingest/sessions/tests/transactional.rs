use super::*;

#[test]
fn completion_failure_rolls_back_raw_rows_ledger_and_cursor() -> anyhow::Result<()> {
    let conn = setup_conn();
    let root = TempRoot::new("completion-rollback");
    let cwd = root.path.to_string_lossy().to_string();
    root.write(
        "proj-a/session-1.jsonl",
        &format!(
            "{}\n",
            claude_line(&cwd, "user", "rollback all file writes")
        ),
    );
    conn.execute_batch(
        "CREATE TRIGGER fail_gh871_cursor
         BEFORE INSERT ON ingest_cursors
         BEGIN
             SELECT RAISE(FAIL, 'forced cursor failure');
         END;",
    )?;

    let summary =
        run_ingest_sessions(&conn, &[root.scan_root("local")], &IngestOptions::default())?;

    assert_eq!(summary.failed_files, 1);
    assert_eq!(summary.ingested_messages, 0);
    assert_eq!(raw_message_count(&conn), 0);
    assert_eq!(cursor_count(&conn), 0);
    assert_eq!(
        conn.query_row(
            "SELECT contract_version FROM raw_session_identities",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        0
    );
    Ok(())
}

#[test]
fn ordinal_replay_conflict_is_sticky_and_preserves_existing_raw_row() -> anyhow::Result<()> {
    let conn = setup_conn();
    let root = TempRoot::new("ordinal-conflict");
    let cwd = root.path.to_string_lossy().to_string();
    let file = root.write(
        "proj-a/session-1.jsonl",
        &format!("{}\n", claude_line(&cwd, "user", "captured value")),
    );
    let scan_root = root.scan_root("local");
    let plan =
        crate::ingest::session_identity::probe(&scan_root.label, &scan_root.path, &file, None)?;
    let identity_id = crate::ingest::session_identity::upsert_claim(&conn, &plan, 1)?;
    crate::ingest::session_identity::resolve_fallback_group(
        &conn,
        plan.host.map(crate::identity::InstallHost::as_db_value),
        &plan.source_root,
        &plan.fallback_session_id,
    )?;
    conn.execute(
        "INSERT INTO raw_messages (
            session_id, project, role, content, content_hash, source,
            created_at_epoch, source_root, event_time_source,
            transcript_identity_id, transcript_record_ordinal
         ) VALUES (?1, ?2, 'assistant', 'different value', ?3, 'transcript',
                   1, ?4, 'transcript_event', ?5, 0)",
        rusqlite::params![
            plan.canonical_session_id,
            plan.project,
            crate::db::content_identity_hash(b"different value"),
            plan.source_root,
            identity_id
        ],
    )?;

    let summary = run_ingest_sessions(&conn, &[scan_root], &IngestOptions::default())?;

    assert_eq!(summary.failed_files, 1);
    assert_eq!(summary.ingested_messages, 0);
    assert_eq!(cursor_count(&conn), 0);
    assert_eq!(
        conn.query_row(
            "SELECT status || ':' || conflict_reason
             FROM raw_session_identities WHERE id = ?1",
            [identity_id],
            |row| row.get::<_, String>(0)
        )?,
        "conflict:stable_occurrence_mismatch"
    );
    assert_eq!(
        conn.query_row(
            "SELECT role || ':' || content FROM raw_messages
             WHERE transcript_identity_id = ?1",
            [identity_id],
            |row| row.get::<_, String>(0)
        )?,
        "assistant:different value"
    );
    Ok(())
}

#[test]
fn fallback_group_conflict_rolls_back_earlier_member_mutations() -> anyhow::Result<()> {
    let conn = setup_conn();
    let root = TempRoot::new("group-atomicity");
    let cwd = root.path.to_string_lossy().to_string();
    let first = root.write(
        "a/shared.jsonl",
        &format!(
            "{}\n",
            serde_json::json!({
                "type": "user",
                "sessionId": "canonical-871",
                "cwd": cwd,
                "timestamp": 100,
                "message": {"content": "first group member"}
            })
        ),
    );
    let second = root.write(
        "b/shared.jsonl",
        &format!(
            "{}\n",
            serde_json::json!({
                "type": "user",
                "sessionId": "canonical-871",
                "cwd": cwd,
                "timestamp": 101,
                "message": {"content": "second group member"}
            })
        ),
    );
    assert!(
        first < second,
        "fixture ordering must exercise earlier mutation"
    );
    let scan_root = root.scan_root("local");
    let second_plan =
        crate::ingest::session_identity::probe(&scan_root.label, &scan_root.path, &second, None)?;
    let second_identity = crate::ingest::session_identity::upsert_claim(&conn, &second_plan, 1)?;
    crate::ingest::session_identity::resolve_fallback_group(
        &conn,
        second_plan
            .host
            .map(crate::identity::InstallHost::as_db_value),
        &second_plan.source_root,
        &second_plan.fallback_session_id,
    )?;
    conn.execute(
        "INSERT INTO raw_messages (
            session_id, project, role, content, content_hash, source,
            created_at_epoch, source_root, event_time_source,
            transcript_identity_id, transcript_record_ordinal
         ) VALUES (?1, ?2, 'assistant', 'preexisting mismatch', ?3, 'transcript',
                   101, ?4, 'transcript_event', ?5, 0)",
        rusqlite::params![
            second_plan.canonical_session_id,
            second_plan.project,
            crate::db::content_identity_hash(b"preexisting mismatch"),
            second_plan.source_root,
            second_identity
        ],
    )?;

    let summary = run_ingest_sessions(&conn, &[scan_root], &IngestOptions::default())?;

    assert_eq!(summary.failed_files, 1);
    assert_eq!(summary.ingested_messages, 0);
    assert_eq!(cursor_count(&conn), 0);
    assert_eq!(raw_message_count(&conn), 1);
    assert_eq!(
        conn.query_row("SELECT content FROM raw_messages", [], |row| row
            .get::<_, String>(0))?,
        "preexisting mismatch"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_session_identities
             WHERE fallback_session_id = 'shared' AND status = 'conflict'",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        2
    );
    Ok(())
}

#[test]
fn filename_fallback_promotion_updates_one_existing_occurrence() -> anyhow::Result<()> {
    let conn = setup_conn();
    let root = TempRoot::new("occurrence-promotion");
    let cwd = root.path.to_string_lossy().to_string();
    let file = root.write(
        "proj-a/fallback-name.jsonl",
        &format!(
            "{}\n",
            serde_json::json!({
                "type": "user",
                "cwd": cwd,
                "timestamp": 100,
                "message": {"content": "same occurrence"}
            })
        ),
    );
    let scan_root = root.scan_root("local");
    let first = run_ingest_sessions(
        &conn,
        std::slice::from_ref(&scan_root),
        &IngestOptions::default(),
    )?;
    assert_eq!(first.ingested_messages, 1);
    let identity_id: i64 = conn.query_row(
        "SELECT transcript_identity_id FROM raw_messages",
        [],
        |row| row.get(0),
    )?;
    std::fs::write(
        &file,
        format!(
            "{}\n",
            serde_json::json!({
                "type": "user",
                "sessionId": "canonical-871",
                "cwd": cwd,
                "timestamp": 100,
                "message": {"content": "same occurrence"}
            })
        ),
    )?;

    let second = run_ingest_sessions(&conn, &[scan_root], &IngestOptions::default())?;

    assert_eq!(second.failed_files, 0);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM raw_messages", [], |row| {
            row.get::<_, i64>(0)
        })?,
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT transcript_identity_id || ':' || session_id
             FROM raw_messages",
            [],
            |row| row.get::<_, String>(0)
        )?,
        format!("{identity_id}:canonical-871")
    );
    Ok(())
}

#[test]
fn raw_sessions_have_created_at_leading_index() {
    let conn = setup_conn();
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'idx_raw_messages_created_source_project_session'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(exists, 1);
}

#[test]
fn scan_root_parse_requires_exact_host_label_path() {
    let parsed = ScanRoot::parse("codex-cli:starlight=/tmp/remote-sessions").unwrap();
    assert_eq!(parsed.host, InstallHost::CodexCli);
    assert_eq!(parsed.label, "starlight");
    assert_eq!(parsed.path, PathBuf::from("/tmp/remote-sessions"));

    assert!(ScanRoot::parse("starlight=/tmp/remote-sessions").is_err());
    assert!(ScanRoot::parse("no-separator").is_err());
    assert!(ScanRoot::parse("unknown:starlight=/tmp").is_err());
    assert!(ScanRoot::parse("codex:starlight=/tmp").is_err());
    let cursor_error = ScanRoot::parse("cursor:archive=/tmp").unwrap_err();
    assert!(cursor_error.to_string().contains("Stop snapshot contract"));
    assert!(ScanRoot::parse("codex-cli:=path-only").is_err());
    assert!(ScanRoot::parse("codex-cli:label=").is_err());
    assert!(ScanRoot::parse("codex-cli:cursor-outcome=/tmp").is_err());
}
