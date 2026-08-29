use super::*;
use rusqlite::Connection;

#[path = "tests/since.rs"]
mod since;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::migrate::run_migrations(&conn).unwrap();
    conn
}

struct TempRoot {
    host: InstallHost,
    path: PathBuf,
    cleanup_root: PathBuf,
}

impl TempRoot {
    fn new(name: &str) -> Self {
        Self::new_with_components(name, &[".claude", "projects"], InstallHost::ClaudeCode)
    }

    fn new_codex(name: &str) -> Self {
        Self::new_with_components(name, &[".codex", "sessions"], InstallHost::CodexCli)
    }

    fn new_unclassified(name: &str) -> Self {
        Self::new_with_components(name, &[], InstallHost::ClaudeCode)
    }

    fn new_with_components(name: &str, components: &[&str], host: InstallHost) -> Self {
        let cleanup_root = std::env::temp_dir().join(format!(
            "remem-ingest-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let path = components
            .iter()
            .fold(cleanup_root.clone(), |path, component| path.join(component));
        std::fs::create_dir_all(&path).unwrap();
        Self {
            host,
            path,
            cleanup_root,
        }
    }

    fn scan_root(&self, label: &str) -> ScanRoot {
        ScanRoot {
            host: self.host,
            label: label.to_string(),
            path: self.path.clone(),
            required: true,
        }
    }

    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.path.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.cleanup_root);
    }
}

fn claude_line(cwd: &str, role: &str, text: &str) -> String {
    format!(
        r#"{{"type":"{role}","cwd":"{cwd}","gitBranch":"main","message":{{"content":[{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

fn raw_message_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM raw_messages", [], |row| row.get(0))
        .unwrap()
}

fn cursor_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM ingest_cursors", [], |row| row.get(0))
        .unwrap()
}

fn run(conn: &Connection, roots: &[ScanRoot]) -> IngestSummary {
    run_ingest_sessions(conn, roots, &IngestOptions::default()).unwrap()
}

#[test]
fn double_run_is_idempotent_and_second_run_skips_via_cursor() {
    let conn = setup_conn();
    let root = TempRoot::new("idempotent");
    let cwd = root.path.to_string_lossy().to_string();
    root.write(
        "proj-a/session-1.jsonl",
        &format!(
            "{}\n{}\n",
            claude_line(&cwd, "user", "first question"),
            claude_line(&cwd, "assistant", "first answer")
        ),
    );
    let roots = [root.scan_root("local")];

    let first = run(&conn, &roots);
    assert_eq!(first.scanned, 1);
    assert_eq!(first.skipped, 0);
    assert_eq!(first.ingested_messages, 2);
    assert_eq!(first.failed_files, 0);
    assert_eq!(first.exit_code(), 0);
    assert_eq!(raw_message_count(&conn), 2);
    assert_eq!(cursor_count(&conn), 1);

    let second = run(&conn, &roots);
    assert_eq!(second.scanned, 1);
    assert_eq!(second.skipped, 1, "unchanged cursor must skip the file");
    assert_eq!(second.ingested_messages, 0);
    assert_eq!(raw_message_count(&conn), 2, "no new rows on second run");
}

#[test]
fn changed_file_is_redrained_and_unique_constraint_dedupes() {
    let conn = setup_conn();
    let root = TempRoot::new("redrain");
    let cwd = root.path.to_string_lossy().to_string();
    let file = root.write(
        "proj-a/session-1.jsonl",
        &format!("{}\n", claude_line(&cwd, "user", "first question")),
    );
    let roots = [root.scan_root("local")];

    let first = run(&conn, &roots);
    assert_eq!(first.ingested_messages, 1);

    // Append a new turn: size changes, cursor invalidates, only the new
    // message inserts.
    let mut content = std::fs::read_to_string(&file).unwrap();
    content.push_str(&claude_line(&cwd, "assistant", "late answer"));
    content.push('\n');
    std::fs::write(&file, content).unwrap();

    let second = run(&conn, &roots);
    assert_eq!(second.skipped, 0);
    assert_eq!(second.ingested_messages, 1, "only appended message is new");
    assert_eq!(raw_message_count(&conn), 2);
}

#[test]
fn corrupt_file_is_isolated_and_batch_continues() {
    let conn = setup_conn();
    let root = TempRoot::new("corrupt");
    let cwd = root.path.to_string_lossy().to_string();
    // Two garbage lines: the non-final parse error counts as a failure even
    // inside the active-tail tolerance window.
    root.write(
        "proj-a/broken.jsonl",
        "not json at all\n{\"also\": broken\n",
    );
    root.write(
        "proj-a/good.jsonl",
        &format!("{}\n", claude_line(&cwd, "user", "healthy message")),
    );
    let roots = [root.scan_root("local")];

    let summary = run(&conn, &roots);
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.failed_files, 1);
    assert_eq!(summary.ingested_messages, 1, "good file still ingests");
    assert_eq!(summary.exit_code(), 1, "partial failure exits non-zero");

    let failures: i64 = conn
        .query_row("SELECT COUNT(*) FROM raw_ingest_failures", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(failures, 1, "corrupt file recorded in raw_ingest_failures");
    // Failed file keeps no cursor, so the next run retries it.
    assert_eq!(cursor_count(&conn), 1);
}

#[test]
fn hostless_legacy_drain_makes_batch_fail_closed_without_duplicates() {
    let conn = setup_conn();
    let root = TempRoot::new("hook-dedup");
    let cwd = root.path.to_string_lossy().to_string();
    let file = root.write(
        "proj-a/session-7.jsonl",
        &format!(
            "{}\n{}\n",
            claude_line(&cwd, "user", "hook question"),
            claude_line(&cwd, "assistant", "hook answer")
        ),
    );

    // Stop-hook path drains first, with the same project identity derivation.
    let project = crate::project_id::project_from_cwd(&cwd);
    let hook_report = raw_archive::drain_transcript(
        &conn,
        &file.to_string_lossy(),
        "session-7",
        &project,
        Some("main"),
        Some(&cwd),
    )
    .unwrap();
    assert_eq!(hook_report.inserted, 2);
    assert_eq!(raw_message_count(&conn), 2);

    let summary = run(&conn, &[root.scan_root("local")]);
    assert_eq!(summary.failed_files, 1);
    assert_eq!(summary.ingested_messages, 0);
    assert_eq!(raw_message_count(&conn), 2);
    assert_eq!(cursor_count(&conn), 0);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_messages WHERE transcript_identity_id IS NULL",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn active_partial_tail_is_not_a_failure_and_keeps_cursor_behind() {
    let conn = setup_conn();
    let root = TempRoot::new("partial-tail");
    let cwd = root.path.to_string_lossy().to_string();
    // Freshly written file (mtime is now) whose last line is a truncated JSON
    // object, as when Claude Code is mid-append.
    root.write(
        "proj-a/live.jsonl",
        &format!(
            "{}\n{}",
            claude_line(&cwd, "user", "complete line"),
            r#"{"type":"assistant","message":{"content":[{"type":"te"#
        ),
    );
    let roots = [root.scan_root("local")];

    let summary = run(&conn, &roots);
    assert_eq!(summary.failed_files, 0, "partial tail is not a failure");
    assert_eq!(summary.partial_files, 1);
    assert_eq!(summary.ingested_messages, 1);
    assert_eq!(summary.exit_code(), 0);
    assert_eq!(cursor_count(&conn), 0, "cursor must not advance");

    let failures: i64 = conn
        .query_row("SELECT COUNT(*) FROM raw_ingest_failures", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(failures, 0, "partial tail is not recorded as a failure");

    // Next run re-reads the tail because no cursor exists for the file.
    let second = run(&conn, &roots);
    assert_eq!(second.skipped, 0);
    assert_eq!(second.ingested_messages, 0);
}

#[test]
fn discovery_excludes_subagents_and_non_jsonl_files() {
    let conn = setup_conn();
    let root = TempRoot::new("discovery");
    let cwd = root.path.to_string_lossy().to_string();
    root.write(
        "proj-a/keep.jsonl",
        &format!("{}\n", claude_line(&cwd, "user", "kept message")),
    );
    root.write(
        "proj-a/subagents/skip.jsonl",
        &format!("{}\n", claude_line(&cwd, "user", "subagent message")),
    );
    root.write("proj-a/notes.txt", "not a transcript\n");

    let summary = run(&conn, &[root.scan_root("local")]);
    assert_eq!(summary.scanned, 1, "only the top-level jsonl is discovered");
    assert_eq!(summary.ingested_messages, 1);
}

#[test]
fn source_root_label_is_stored_and_distinguishes_roots() {
    let conn = setup_conn();
    let root_a = TempRoot::new("root-a");
    let root_b = TempRoot::new("root-b");
    // No cwd in these lines: project falls back to the directory slug, which
    // collides across roots; source_root must still tell them apart.
    let line =
        r#"{"type":"user","message":{"content":[{"type":"text","text":"same project name"}]}}"#;
    root_a.write("proj-x/session-same.jsonl", &format!("{line}\n"));
    root_b.write("proj-x/session-same.jsonl", &format!("{line}\n"));

    let summary = run(
        &conn,
        &[root_a.scan_root("local"), root_b.scan_root("starlight")],
    );
    assert_eq!(summary.ingested_messages, 2);

    let labels: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT source_root, project FROM raw_messages ORDER BY source_root")
            .unwrap();
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    };
    assert_eq!(
        labels,
        vec![
            ("local".to_string(), "proj-x".to_string()),
            ("starlight".to_string(), "proj-x".to_string()),
        ]
    );
}

#[test]
fn same_fallback_filename_isolated_across_batch_hosts() -> anyhow::Result<()> {
    let conn = setup_conn();
    let claude_root = TempRoot::new("cross-host-claude");
    let codex_root = TempRoot::new_codex("cross-host-codex");
    let claude_cwd = claude_root.path.to_string_lossy().to_string();
    let codex_cwd = codex_root.path.to_string_lossy().to_string();
    claude_root.write(
        "repo/shared.jsonl",
        &format!(
            "{}\n",
            serde_json::json!({
                "type": "user",
                "sessionId": "claude-canonical",
                "cwd": claude_cwd,
                "timestamp": 100,
                "message": {"content": "claude message"}
            })
        ),
    );
    codex_root.write(
        "repo/shared.jsonl",
        &format!(
            "{}\n",
            serde_json::json!({
                "type": "user",
                "sessionId": "codex-canonical",
                "cwd": codex_cwd,
                "timestamp": 101,
                "message": {"content": "codex message"}
            })
        ),
    );

    let summary = run_ingest_sessions(
        &conn,
        &[
            claude_root.scan_root("local"),
            codex_root.scan_root("local"),
        ],
        &IngestOptions::default(),
    )?;

    assert_eq!(summary.failed_files, 0);
    assert_eq!(summary.ingested_messages, 2);
    let identities = {
        let mut statement = conn.prepare(
            "SELECT host, status, canonical_session_id
             FROM raw_session_identities ORDER BY host",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    assert_eq!(
        identities,
        vec![
            (
                "claude-code".to_string(),
                "active".to_string(),
                "claude-canonical".to_string(),
            ),
            (
                "codex-cli".to_string(),
                "active".to_string(),
                "codex-canonical".to_string(),
            ),
        ]
    );
    Ok(())
}

#[test]
fn cross_host_fallback_cannot_claim_unresolved_legacy_row() -> anyhow::Result<()> {
    let conn = setup_conn();
    let claude_root = TempRoot::new("cross-host-legacy-claude");
    let codex_root = TempRoot::new_codex("cross-host-legacy-codex");
    let line = |session_id: &str| {
        format!(
            "{}\n",
            serde_json::json!({
                "type": "user",
                "sessionId": session_id,
                "timestamp": 100,
                "message": {"content": "shared legacy content"}
            })
        )
    };
    claude_root.write("repo/shared.jsonl", &line("claude-canonical"));
    codex_root.write("repo/shared.jsonl", &line("codex-canonical"));
    conn.execute(
        "INSERT INTO raw_messages (
            id, session_id, project, role, content, content_hash, source,
            created_at_epoch, source_root, event_time_source
         ) VALUES (41, 'shared', 'repo', 'user', 'shared legacy content', ?1,
                   'transcript', 100, 'local', 'legacy_unknown')",
        [crate::db::content_identity_hash(b"shared legacy content")],
    )?;

    let summary = run_ingest_sessions(
        &conn,
        &[
            claude_root.scan_root("local"),
            codex_root.scan_root("local"),
        ],
        &IngestOptions::default(),
    )?;

    assert_eq!(summary.failed_files, 2);
    assert_eq!(summary.ingested_messages, 0);
    assert_eq!(raw_message_count(&conn), 1);
    assert_eq!(
        conn.query_row(
            "SELECT session_id || ':' || project
             FROM raw_messages WHERE id = 41 AND transcript_identity_id IS NULL",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "shared:repo"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_session_identities WHERE status = 'conflict'",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        2
    );
    Ok(())
}

#[test]
fn cursor_key_includes_source_root_label() {
    let conn = setup_conn();
    let root = TempRoot::new("cursor-source-root");
    let line =
        r#"{"type":"user","message":{"content":[{"type":"text","text":"same synced file"}]}}"#;
    root.write("proj-x/session-same.jsonl", &format!("{line}\n"));

    let first = run(&conn, &[root.scan_root("local")]);
    assert_eq!(first.ingested_messages, 1);
    assert_eq!(cursor_count(&conn), 1);

    let second = run(&conn, &[root.scan_root("starlight")]);
    assert_eq!(
        second.skipped, 0,
        "same path under a different source_root must not use the old cursor"
    );
    assert_eq!(second.ingested_messages, 1);
    assert_eq!(raw_message_count(&conn), 2);
    assert_eq!(cursor_count(&conn), 2);

    let labels: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT source_root FROM raw_messages ORDER BY source_root")
            .unwrap();
        let rows = stmt.query_map([], |row| row.get(0)).unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    };
    assert_eq!(labels, vec!["local".to_string(), "starlight".to_string()]);
}

#[test]
fn codex_session_meta_id_overrides_rollout_filename() {
    let conn = setup_conn();
    let root = TempRoot::new_codex("codex-session-id");
    root.write(
        "2026/06/12/rollout-abc.jsonl",
        include_str!("../../../tests/fixtures/codex-rollout-minimal.jsonl"),
    );

    let summary = run(&conn, &[root.scan_root("local")]);
    assert_eq!(summary.failed_files, 0);
    assert_eq!(summary.ingested_messages, 2);

    let session_id: String = conn
        .query_row("SELECT DISTINCT session_id FROM raw_messages", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(session_id, "019eba00-sanitized");
}

#[test]
fn transcript_timestamps_drive_raw_message_window_time() {
    let conn = setup_conn();
    let root = TempRoot::new("transcript-time");
    root.write(
        "proj-a/session-1.jsonl",
        r#"{"timestamp":"2026-06-12T00:00:01.000Z","type":"user","message":{"content":[{"type":"text","text":"historical question"}]}}"#,
    );

    let summary = run(&conn, &[root.scan_root("local")]);
    assert_eq!(summary.failed_files, 0);
    assert_eq!(summary.ingested_messages, 1);

    let created_at_epoch: i64 = conn
        .query_row("SELECT created_at_epoch FROM raw_messages", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(created_at_epoch, 1_781_222_401);
}

#[test]
fn missing_explicit_root_is_reported_as_failure() {
    let conn = setup_conn();
    let missing = std::env::temp_dir().join(format!(
        "remem-ingest-missing-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));

    let summary = run_ingest_sessions(
        &conn,
        &[ScanRoot {
            host: InstallHost::CodexCli,
            label: "remote".to_string(),
            path: missing,
            required: true,
        }],
        &IngestOptions::default(),
    )
    .unwrap();

    assert_eq!(summary.failed_files, 1);
    assert_eq!(summary.exit_code(), 1);
}

#[test]
fn constructed_reserved_source_root_fails_before_ingest_mutation() -> anyhow::Result<()> {
    let conn = setup_conn();
    let root = TempRoot::new("reserved-source-root");
    let cwd = root.path.to_string_lossy().to_string();
    root.write(
        "proj-a/session-1.jsonl",
        &format!("{}\n", claude_line(&cwd, "user", "must not ingest")),
    );

    let error = run_ingest_sessions(
        &conn,
        &[root.scan_root(RESERVED_CURSOR_OUTCOME_SOURCE_ROOT)],
        &IngestOptions::default(),
    )
    .expect_err("reserved source root must fail before discovery or mutation");

    assert!(error.to_string().contains("reserved"));
    assert_eq!(raw_message_count(&conn), 0);
    assert_eq!(cursor_count(&conn), 0);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM raw_session_identities", [], |row| {
            row.get::<_, i64>(0)
        })?,
        0
    );
    Ok(())
}

#[test]
fn incomplete_discovery_blocks_all_identity_and_raw_mutation() -> anyhow::Result<()> {
    let conn = setup_conn();
    let root = TempRoot::new("phase-a-fail-closed");
    let cwd = root.path.to_string_lossy().to_string();
    root.write(
        "proj-a/good.jsonl",
        &format!("{}\n", claude_line(&cwd, "user", "must not ingest")),
    );
    let missing = root.path.join("missing-required-root");
    let roots = [
        root.scan_root("local"),
        ScanRoot {
            host: InstallHost::ClaudeCode,
            label: "missing".to_string(),
            path: missing,
            required: true,
        },
    ];

    let summary = run_ingest_sessions(&conn, &roots, &IngestOptions::default())?;

    assert_eq!(summary.failed_files, 1);
    assert_eq!(summary.ingested_messages, 0);
    assert_eq!(raw_message_count(&conn), 0);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM raw_session_identities", [], |row| {
            row.get::<_, i64>(0)
        })?,
        0
    );
    assert_eq!(cursor_count(&conn), 0);
    Ok(())
}

#[test]
fn incomplete_probe_blocks_all_phase_a_mutation() -> anyhow::Result<()> {
    let conn = setup_conn();
    let root = TempRoot::new("phase-a-probe-fail");
    let cwd = root.path.to_string_lossy().to_string();
    root.write(
        "proj-a/good.jsonl",
        &format!("{}\n", claude_line(&cwd, "user", "must not ingest")),
    );
    let broken = root.path.join("proj-a/broken.jsonl");
    std::fs::write(&broken, [0xff, b'\n'])?;

    let summary =
        run_ingest_sessions(&conn, &[root.scan_root("local")], &IngestOptions::default())?;

    assert_eq!(summary.failed_files, 1);
    assert_eq!(raw_message_count(&conn), 0);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM raw_session_identities", [], |row| {
            row.get::<_, i64>(0)
        })?,
        0
    );
    Ok(())
}

#[test]
fn explicit_host_ingests_unclassified_transcript_path() -> anyhow::Result<()> {
    let conn = setup_conn();
    let root = TempRoot::new_unclassified("unknown-host");
    let cwd = root.path.to_string_lossy().to_string();
    root.write(
        "proj-a/session-1.jsonl",
        &format!("{}\n", claude_line(&cwd, "user", "explicit host")),
    );

    let summary =
        run_ingest_sessions(&conn, &[root.scan_root("local")], &IngestOptions::default())?;

    assert_eq!(summary.failed_files, 0);
    assert_eq!(summary.ingested_messages, 1);
    assert_eq!(raw_message_count(&conn), 1);
    assert_eq!(
        conn.query_row("SELECT host FROM raw_session_identities", [], |row| {
            row.get::<_, String>(0)
        })?,
        "claude-code"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn unreadable_discovery_entry_is_isolated_and_batch_continues() {
    use std::os::unix::fs::PermissionsExt;

    let conn = setup_conn();
    let root = TempRoot::new("discovery-isolation");
    let cwd = root.path.to_string_lossy().to_string();
    root.write(
        "proj-a/good.jsonl",
        &format!("{}\n", claude_line(&cwd, "user", "healthy message")),
    );
    let blocked = root.path.join("proj-a").join("blocked");
    std::fs::create_dir_all(&blocked).unwrap();
    let original_permissions = std::fs::metadata(&blocked).unwrap().permissions();
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();

    let summary = run(&conn, &[root.scan_root("local")]);

    std::fs::set_permissions(&blocked, original_permissions).unwrap();

    assert_eq!(summary.ingested_messages, 0);
    assert_eq!(summary.failed_files, 1);
    assert_eq!(raw_message_count(&conn), 0);
}

#[path = "tests/transactional.rs"]
mod transactional;
