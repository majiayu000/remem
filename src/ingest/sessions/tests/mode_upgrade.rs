use super::*;

fn state(conn: &Connection) -> Vec<(i64, String, i64, i64)> {
    conn.prepare(
        "SELECT id, session_mode, session_mode_version, contract_version
        FROM raw_session_identities ORDER BY transcript_path",
    )
    .unwrap()
    .query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}
fn transcript(source: &str, originator: &str) -> String {
    format!(
        "{}\n{}\n",
        serde_json::json!({"type":"session_meta", "payload": {
        "source":source, "originator":originator}}),
        serde_json::json!({"type":"response_item", "payload": {"type":"message", "role":"user", "content":"synthetic"}})
    )
}
fn legacy(conn: &Connection, path: &std::path::Path, mode: &str) {
    conn.execute("UPDATE raw_session_identities SET session_mode=?1, session_mode_version=0 WHERE transcript_path=?2",
        rusqlite::params![mode,path.to_string_lossy()]).unwrap();
}

// Catches old known modes rolling back whole batches and cursor skips hiding upgrades.
#[test]
fn legacy_modes_upgrade_before_unchanged_cursor_without_duplicate_raw_rows() {
    let conn = setup_conn();
    let root = TempRoot::new_codex("mode-upgrade");
    let exec = root.write("a-exec.jsonl", &transcript("exec", "Codex Desktop"));
    let sub = root.write("b-sub.jsonl", &transcript("subagent", "codex-tui"));
    let same = root.write("c-same.jsonl", &transcript("cli", "codex-tui"));
    assert_eq!(run(&conn, &[root.scan_root("local")]).ingested_messages, 3);
    let expected = state(&conn);
    for path in [&exec, &sub, &same] {
        legacy(&conn, path, "interactive");
    }
    let upgraded = run(&conn, &[root.scan_root("local")]);
    assert_eq!(
        upgraded,
        IngestSummary {
            scanned: 3,
            skipped: 3,
            ..Default::default()
        }
    );
    assert_eq!(state(&conn), expected);
    assert_eq!(
        expected
            .iter()
            .map(|r| (&*r.1, r.2, r.3))
            .collect::<Vec<_>>(),
        vec![
            ("unattended", 1, 1),
            ("subagent", 1, 1),
            ("interactive", 1, 1)
        ]
    );
    assert_eq!((raw_message_count(&conn), cursor_count(&conn)), (3, 3));
    assert_eq!(run(&conn, &[root.scan_root("local")]), upgraded);
    assert_eq!(state(&conn), expected);
}

// Catches committing the first upgrade when a later legacy row has real provenance drift.
#[test]
fn conflicting_legacy_evidence_rolls_back_all_mode_versions_and_raw_mutation() {
    let conn = setup_conn();
    let root = TempRoot::new_codex("mode-rollback");
    let first = root.write("a-upgrade.jsonl", &transcript("exec", "Codex Desktop"));
    let last = root.write("z-conflict.jsonl", &transcript("exec", "codex_exec"));
    run(&conn, &[root.scan_root("local")]);
    legacy(&conn, &first, "interactive");
    legacy(&conn, &last, "interactive");
    let before = state(&conn);
    root.write("b-new.jsonl", &transcript("cli", "codex-tui"));
    let error = run_ingest_sessions(&conn, &[root.scan_root("local")], &IngestOptions::default())
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("session-mode provenance conflict"),
        "{error:#}"
    );
    assert_eq!(state(&conn), before);
    assert_eq!((raw_message_count(&conn), cursor_count(&conn)), (2, 2));
}

// Catches treating every future source change as another classifier upgrade.
#[test]
fn shared_version_rejects_source_only_mode_change() {
    let conn = setup_conn();
    let root = TempRoot::new_codex("mode-source-change");
    root.write("s.jsonl", &transcript("cli", "Codex Desktop"));
    run(&conn, &[root.scan_root("local")]);
    let before = state(&conn);
    root.write("s.jsonl", &transcript("exec", "Codex Desktop"));
    let error = run_ingest_sessions(&conn, &[root.scan_root("local")], &IngestOptions::default())
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("session-mode provenance conflict"),
        "{error:#}"
    );
    assert_eq!(state(&conn), before);
    assert_eq!((raw_message_count(&conn), cursor_count(&conn)), (1, 1));
}
