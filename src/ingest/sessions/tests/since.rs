use super::*;

#[test]
fn since_bound_skips_older_files() {
    let conn = setup_conn();
    let root = TempRoot::new("since");
    let cwd = root.path.to_string_lossy().to_string();
    root.write(
        "proj-a/session-1.jsonl",
        &format!("{}\n", claude_line(&cwd, "user", "recent message")),
    );
    let roots = [root.scan_root("local")];

    let future = chrono::Utc::now().timestamp() + 3600;
    let summary = run_ingest_sessions(
        &conn,
        &roots,
        &IngestOptions {
            since_epoch: Some(future),
        },
    )
    .unwrap();
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.skipped, 1);
    assert_eq!(summary.ingested_messages, 0);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM raw_session_identities", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        1,
        "Phase A must claim files that Phase B skips by --since"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_session_identity_claims",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn since_bound_reports_new_identity_conflict_as_failure() {
    let conn = setup_conn();
    let root = TempRoot::new("since-conflict");
    let cwd = root.path.to_string_lossy().to_string();
    let transcript = root.write(
        "proj-a/session-1.jsonl",
        &format!(
            "{}\n",
            serde_json::json!({
                "type": "user",
                "sessionId": "canonical-a",
                "cwd": cwd,
                "message": {"content": "first claim"}
            })
        ),
    );
    let roots = [root.scan_root("local")];
    let first = run(&conn, &roots);
    assert_eq!(first.failed_files, 0);

    std::fs::write(
        transcript,
        format!(
            "{}\n",
            serde_json::json!({
                "type": "user",
                "sessionId": "canonical-b",
                "cwd": cwd,
                "message": {"content": "conflicting claim"}
            })
        ),
    )
    .unwrap();
    let future = chrono::Utc::now().timestamp() + 3600;
    let summary = run_ingest_sessions(
        &conn,
        &roots,
        &IngestOptions {
            since_epoch: Some(future),
        },
    )
    .unwrap();

    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.failed_files, 1);
    assert_eq!(summary.exit_code(), 1);
    assert_eq!(
        conn.query_row(
            "SELECT status || ':' || conflict_reason
             FROM raw_session_identities",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "conflict:conflicting_metadata_claims"
    );
}
