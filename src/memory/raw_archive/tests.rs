use super::*;
use crate::memory::raw_query::{parse_time_lower_bound, parse_time_upper_bound};

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::migrate::run_migrations(&conn).unwrap();
    conn
}

fn write_temp_transcript(name: &str, content: &str) -> Result<std::path::PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "remem-{name}-{}-{}.jsonl",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::write(&path, content)?;
    Ok(path)
}

fn raw_ingest_failure_count(conn: &Connection) -> Result<i64> {
    Ok(
        conn.query_row("SELECT COUNT(*) FROM raw_ingest_failures", [], |row| {
            row.get(0)
        })?,
    )
}

#[test]
fn insert_is_idempotent_per_session_role_content() -> Result<()> {
    let conn = setup_conn();
    let id1 = insert_raw_message(
        &conn,
        "s1",
        "/proj",
        ROLE_USER,
        "hello world",
        SOURCE_HOOK,
        None,
        None,
    )?
    .ok_or_else(|| anyhow::anyhow!("first insert returned None"))?;
    // Same session + same text => deduped onto the existing row.
    let id2 = insert_raw_message(
        &conn,
        "s1",
        "/proj",
        ROLE_USER,
        "hello world",
        SOURCE_HOOK,
        None,
        None,
    )?
    .ok_or_else(|| anyhow::anyhow!("second insert returned None"))?;
    assert_eq!(id1.id, id2.id);
    assert!(id1.inserted, "first call must mark inserted");
    assert!(!id2.inserted, "second call must mark not-inserted");
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM raw_messages", [], |row| row.get(0))?;
    assert_eq!(count, 1);
    let stored_hash: String = conn.query_row(
        "SELECT content_hash FROM raw_messages WHERE id = ?1",
        params![id1.id],
        |row| row.get(0),
    )?;
    assert!(stored_hash.starts_with("sha256:content-v1:"));
    assert_eq!(stored_hash.len(), "sha256:content-v1:".len() + 64);
    Ok(())
}

#[test]
fn insert_reuses_matching_legacy_content_hash() -> Result<()> {
    let conn = setup_conn();
    let content = "legacy exact raw message";
    let legacy_hash = legacy_exact_content_hash(content);
    conn.execute(
        "INSERT INTO raw_messages
         (session_id, project, role, content, content_hash, source, branch, cwd, created_at_epoch)
         VALUES ('s1', '/proj', ?1, ?2, ?3, ?4, NULL, NULL, 100)",
        params![ROLE_USER, content, legacy_hash, SOURCE_HOOK],
    )?;
    let legacy_id = conn.last_insert_rowid();

    let outcome = insert_raw_message(
        &conn,
        "s1",
        "/proj",
        ROLE_USER,
        content,
        SOURCE_HOOK,
        None,
        None,
    )?
    .ok_or_else(|| anyhow::anyhow!("non-empty content returned None"))?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM raw_messages", [], |row| row.get(0))?;

    assert_eq!(outcome.id, legacy_id);
    assert!(!outcome.inserted);
    assert_eq!(count, 1);
    Ok(())
}

#[test]
fn insert_does_not_reuse_mismatched_legacy_content_hash() -> Result<()> {
    let conn = setup_conn();
    let content = "target raw content";
    let legacy_hash = legacy_exact_content_hash(content);
    conn.execute(
        "INSERT INTO raw_messages
         (session_id, project, role, content, content_hash, source, branch, cwd, created_at_epoch)
         VALUES ('s1', '/proj', ?1, 'different raw content', ?2, ?3, NULL, NULL, 100)",
        params![ROLE_USER, legacy_hash, SOURCE_HOOK],
    )?;

    let outcome = insert_raw_message(
        &conn,
        "s1",
        "/proj",
        ROLE_USER,
        content,
        SOURCE_HOOK,
        None,
        None,
    )?
    .ok_or_else(|| anyhow::anyhow!("non-empty content returned None"))?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM raw_messages", [], |row| row.get(0))?;
    let stored_hash: String = conn.query_row(
        "SELECT content_hash FROM raw_messages WHERE id = ?1",
        params![outcome.id],
        |row| row.get(0),
    )?;

    assert!(outcome.inserted);
    assert_eq!(count, 2);
    assert!(stored_hash.starts_with("sha256:content-v1:"));
    Ok(())
}

/// Regression for #237: the same text spoken in two different sessions must
/// keep BOTH turns. The old UNIQUE(project, role, content_hash) globally
/// deduped across sessions and silently dropped the second turn.
#[test]
fn identical_text_across_sessions_keeps_both_turns() {
    let conn = setup_conn();
    let id1 = insert_raw_message(
        &conn,
        "s1",
        "/proj",
        ROLE_USER,
        "let's deploy the service",
        SOURCE_HOOK,
        None,
        None,
    )
    .unwrap()
    .expect("first insert returns Some");
    let id2 = insert_raw_message(
        &conn,
        "s2",
        "/proj",
        ROLE_USER,
        "let's deploy the service",
        SOURCE_HOOK,
        None,
        None,
    )
    .unwrap()
    .expect("second insert returns Some");

    assert!(id1.inserted, "first session turn must be inserted");
    assert!(id2.inserted, "second session turn must also be inserted");
    assert_ne!(id1.id, id2.id, "the two sessions must keep distinct rows");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM raw_messages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2, "both session turns must be preserved");
}

#[test]
fn empty_content_is_skipped() {
    let conn = setup_conn();
    let id = insert_raw_message(
        &conn,
        "s1",
        "/proj",
        ROLE_USER,
        "   \n\t  ",
        SOURCE_HOOK,
        None,
        None,
    )
    .unwrap();
    assert!(id.is_none());
}

#[test]
fn fts_finds_inserted_content() {
    let conn = setup_conn();
    insert_raw_message(
        &conn,
        "s1",
        "/proj",
        ROLE_USER,
        "帮我看看 VPS RackNerd 的价格",
        SOURCE_HOOK,
        None,
        None,
    )
    .unwrap();
    let hits = search_raw_messages(
        &conn,
        &RawSearchRequest {
            query: "RackNerd".to_string(),
            project: Some("/proj".to_string()),
            branch: None,
            role: None,
            limit: 10,
            offset: 0,
            since_epoch: None,
            until_epoch: None,
        },
    )
    .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].content.contains("RackNerd"));
}

#[test]
fn search_branch_filter_keeps_matching_and_branchless_raw_messages() {
    let conn = setup_conn();
    insert_raw_message(
        &conn,
        "s-main",
        "/proj",
        ROLE_USER,
        "shared needle on main",
        SOURCE_HOOK,
        Some("main"),
        None,
    )
    .unwrap();
    insert_raw_message(
        &conn,
        "s-feature",
        "/proj",
        ROLE_USER,
        "shared needle on feature",
        SOURCE_HOOK,
        Some("feature"),
        None,
    )
    .unwrap();
    insert_raw_message(
        &conn,
        "s-branchless",
        "/proj",
        ROLE_USER,
        "shared needle without branch",
        SOURCE_HOOK,
        None,
        None,
    )
    .unwrap();

    let hits = search_raw_messages(
        &conn,
        &RawSearchRequest {
            query: "needle".to_string(),
            project: Some("/proj".to_string()),
            branch: Some("main".to_string()),
            role: None,
            limit: 10,
            offset: 0,
            since_epoch: None,
            until_epoch: None,
        },
    )
    .unwrap();
    let branches: Vec<Option<String>> = hits.into_iter().map(|hit| hit.branch).collect();

    assert!(branches.contains(&Some("main".to_string())));
    assert!(branches.contains(&None));
    assert!(
        !branches.contains(&Some("feature".to_string())),
        "{branches:?}"
    );
}

#[test]
fn drain_transcript_honors_captured_byte_limit() -> Result<()> {
    let conn = setup_conn();
    let first =
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"before stop"}]}}"#;
    let second =
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"after stop"}]}}"#;
    let path = write_temp_transcript("raw-byte-limit", &format!("{first}\n{second}\n"))?;
    let options = TranscriptDrainOptions::default();

    let report = drain_transcript_with_capture_limit(
        &conn,
        path.to_string_lossy().as_ref(),
        "session-byte-limit",
        "/proj",
        None,
        None,
        &options,
        Some((first.len() + 1) as u64),
    )?;

    assert_eq!(report.inserted, 1);
    let contents: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT content FROM raw_messages WHERE session_id = 'session-byte-limit' ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    assert_eq!(contents, vec!["before stop".to_string()]);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn drain_transcript_rejects_content_truncated_before_captured_boundary() -> Result<()> {
    let conn = setup_conn();
    let content = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"kept"}]}}"#;
    let path = write_temp_transcript("raw-truncated-boundary", content)?;
    let options = TranscriptDrainOptions::default();

    let report = drain_transcript_with_capture_limit(
        &conn,
        path.to_string_lossy().as_ref(),
        "session-truncated-boundary",
        "/proj",
        None,
        None,
        &options,
        Some(content.len() as u64 + 10),
    )?;

    assert!(report.read_error.is_some());
    assert_eq!(report.inserted, 0);
    assert_eq!(raw_ingest_failure_count(&conn)?, 1);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn drain_transcript_counts_parse_errors_and_records_failure() -> Result<()> {
    let conn = setup_conn();
    let path = write_temp_transcript(
        "raw-parse-error",
        format!(
            "{}\nnot json\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"kept message"}]}}"#
        )
        .as_str(),
    )?;

    let report = drain_transcript(
        &conn,
        path.to_string_lossy().as_ref(),
        "session-parse",
        "/proj",
        None,
        None,
    )?;

    assert_eq!(report.inserted, 1);
    assert_eq!(report.parse_errors, 1);
    assert_eq!(raw_ingest_failure_count(&conn)?, 1);
    let (kind, parse_errors): (String, i64) = conn.query_row(
        "SELECT error_kind, parse_errors FROM raw_ingest_failures",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(kind, "parse_errors");
    assert_eq!(parse_errors, 1);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn drain_transcript_counts_insert_errors_and_records_failure() -> Result<()> {
    let conn = setup_conn();
    conn.execute_batch(
        "CREATE TRIGGER fail_raw_archive_insert
         BEFORE INSERT ON raw_messages
         BEGIN
             SELECT RAISE(FAIL, 'raw insert failed');
         END;",
    )?;
    let path = write_temp_transcript(
        "raw-insert-error",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"cannot insert"}]}}"#,
    )?;

    let report = drain_transcript(
        &conn,
        path.to_string_lossy().as_ref(),
        "session-insert",
        "/proj",
        None,
        None,
    )?;

    assert_eq!(report.inserted, 0);
    assert_eq!(report.insert_errors, 1);
    assert_eq!(raw_ingest_failure_count(&conn)?, 1);
    let (kind, insert_errors): (String, i64) = conn.query_row(
        "SELECT error_kind, insert_errors FROM raw_ingest_failures",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(kind, "insert_errors");
    assert_eq!(insert_errors, 1);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn drain_transcript_savepoint_can_run_inside_outer_transaction() -> Result<()> {
    let conn = setup_conn();
    let path = write_temp_transcript(
        "raw-nested-savepoint",
        concat!(
            r#"{"type":"user","message":{"content":[{"type":"text","text":"outer transaction user"}]}}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"outer transaction assistant"}]}}"#,
            "\n"
        ),
    )?;

    conn.execute_batch("BEGIN IMMEDIATE;")?;
    let report = drain_transcript(
        &conn,
        path.to_string_lossy().as_ref(),
        "session-nested",
        "/proj",
        None,
        None,
    )?;

    assert_eq!(report.inserted, 2);
    let count_inside: i64 = conn.query_row(
        "SELECT COUNT(*) FROM raw_messages WHERE session_id = 'session-nested'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(count_inside, 2);

    conn.execute_batch("ROLLBACK;")?;
    let count_after: i64 = conn.query_row(
        "SELECT COUNT(*) FROM raw_messages WHERE session_id = 'session-nested'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(count_after, 0);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn drain_transcript_parses_codex_rollout_response_items() -> Result<()> {
    let conn = setup_conn();
    let path = write_temp_transcript(
        "codex-rollout",
        include_str!("../../../tests/fixtures/codex-rollout-minimal.jsonl"),
    )?;

    let report = drain_transcript(
        &conn,
        path.to_string_lossy().as_ref(),
        "codex-session",
        "/proj",
        None,
        Some("/tmp/remem-codex-fixture"),
    )?;

    assert_eq!(report.inserted, 2, "{report:?}");
    assert_eq!(report.parse_errors, 0);
    assert_eq!(report.insert_errors, 0);

    let rows = search_raw_messages(
        &conn,
        &RawSearchRequest {
            query: "Codex rollout".to_string(),
            project: Some("/proj".to_string()),
            branch: None,
            role: None,
            limit: 10,
            offset: 0,
            since_epoch: None,
            until_epoch: None,
        },
    )?;
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.iter().any(|row| row.role == ROLE_USER));
    assert!(rows.iter().any(|row| row.role == ROLE_ASSISTANT));
    assert!(rows.iter().all(|row| row.source == SOURCE_TRANSCRIPT
        && row.cwd.as_deref() == Some("/tmp/remem-codex-fixture")));
    std::fs::remove_file(path)?;
    Ok(())
}

fn insert_at_epoch(
    conn: &Connection,
    session_id: &str,
    project: &str,
    role: &str,
    content: &str,
    epoch: i64,
) {
    let outcome = insert_raw_message(
        conn,
        session_id,
        project,
        role,
        content,
        SOURCE_HOOK,
        None,
        None,
    )
    .unwrap()
    .expect("insert must produce a row");
    assert!(outcome.inserted);
    conn.execute(
        "UPDATE raw_messages SET created_at_epoch = ?1 WHERE id = ?2",
        params![epoch, outcome.id],
    )
    .unwrap();
}

#[test]
fn search_without_window_matches_pre_window_behavior() {
    let conn = setup_conn();
    insert_at_epoch(&conn, "s1", "/proj", ROLE_USER, "needle early", 100);
    insert_at_epoch(&conn, "s1", "/proj", ROLE_USER, "needle late", 900);

    let base = RawSearchRequest {
        query: "needle".to_string(),
        project: Some("/proj".to_string()),
        branch: None,
        role: None,
        limit: 10,
        offset: 0,
        since_epoch: None,
        until_epoch: None,
    };
    let hits = search_raw_messages(&conn, &base).unwrap();
    assert_eq!(hits.len(), 2, "None window returns everything");
}

#[test]
fn search_window_filters_by_created_at_epoch() {
    let conn = setup_conn();
    insert_at_epoch(&conn, "s1", "/proj", ROLE_USER, "needle early", 100);
    insert_at_epoch(&conn, "s1", "/proj", ROLE_USER, "needle middle", 500);
    insert_at_epoch(&conn, "s1", "/proj", ROLE_USER, "needle late", 900);

    let request = RawSearchRequest {
        query: "needle".to_string(),
        project: Some("/proj".to_string()),
        branch: None,
        role: None,
        limit: 10,
        offset: 0,
        since_epoch: Some(200),
        until_epoch: Some(800),
    };
    let hits = search_raw_messages(&conn, &request).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].content.contains("middle"));

    let since_only = RawSearchRequest {
        since_epoch: Some(500),
        until_epoch: None,
        ..request.clone()
    };
    let hits = search_raw_messages(&conn, &since_only).unwrap();
    assert_eq!(hits.len(), 2, "since bound is inclusive");
}

#[test]
fn list_sessions_groups_by_root_project_session_with_window_bounds() {
    let conn = setup_conn();
    insert_at_epoch(&conn, "s1", "/proj", ROLE_USER, "s1 q1", 100);
    insert_at_epoch(&conn, "s1", "/proj", ROLE_ASSISTANT, "s1 a1", 150);
    insert_at_epoch(&conn, "s2", "/proj", ROLE_USER, "s2 q1", 300);
    insert_at_epoch(&conn, "s3", "/other", ROLE_USER, "s3 q1", 200);
    // Outside the window: excluded from grouping entirely.
    insert_at_epoch(&conn, "s4", "/proj", ROLE_USER, "too old", 10);

    let sessions = list_sessions(
        &conn,
        &RawSessionQuery {
            since_epoch: Some(50),
            until_epoch: Some(1000),
            project: None,
            sample_user_messages: 0,
        },
    )
    .unwrap();
    assert_eq!(sessions.len(), 3);
    assert_eq!(sessions[0].session_id, "s1");
    assert_eq!(sessions[0].first_epoch, 100);
    assert_eq!(sessions[0].last_epoch, 150);
    assert_eq!(sessions[0].message_count, 2);
    assert_eq!(sessions[0].source_root, "local");
    assert_eq!(sessions[1].session_id, "s3", "ordered by first epoch");
    assert_eq!(sessions[2].session_id, "s2");

    let filtered = list_sessions(
        &conn,
        &RawSessionQuery {
            since_epoch: Some(50),
            until_epoch: Some(1000),
            project: Some("/proj".to_string()),
            sample_user_messages: 0,
        },
    )
    .unwrap();
    assert_eq!(filtered.len(), 2, "project filter applies");
}

#[test]
fn list_sessions_samples_first_user_messages_in_window_order() {
    let conn = setup_conn();
    insert_at_epoch(&conn, "s1", "/proj", ROLE_USER, "first question", 100);
    insert_at_epoch(&conn, "s1", "/proj", ROLE_ASSISTANT, "an answer", 110);
    insert_at_epoch(&conn, "s1", "/proj", ROLE_USER, "second question", 120);
    insert_at_epoch(&conn, "s1", "/proj", ROLE_USER, "third question", 130);
    let long = "x".repeat(400);
    insert_at_epoch(&conn, "s2", "/proj", ROLE_USER, &long, 200);

    let sessions = list_sessions(
        &conn,
        &RawSessionQuery {
            since_epoch: None,
            until_epoch: None,
            project: Some("/proj".to_string()),
            sample_user_messages: 2,
        },
    )
    .unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions[0].user_message_samples,
        vec!["first question".to_string(), "second question".to_string()],
        "only role=user, ascending, capped at N"
    );
    assert_eq!(
        sessions[1].user_message_samples[0].chars().count(),
        200,
        "samples are truncated"
    );
}

#[test]
fn parse_time_bounds_accept_epoch_iso_and_date() {
    assert_eq!(parse_time_lower_bound("1750000000").unwrap(), 1_750_000_000);
    assert_eq!(
        parse_time_upper_bound("2026-01-02T03:04:05Z").unwrap(),
        1_767_323_045
    );
    assert_eq!(parse_time_lower_bound("2026-01-02").unwrap(), 1_767_312_000);
    assert!(parse_time_lower_bound("not-a-time").is_err());
    assert!(parse_time_upper_bound("not-a-time").is_err());
}

#[test]
fn date_only_until_bound_includes_the_full_utc_day() {
    assert_eq!(
        parse_time_upper_bound("2026-01-02").unwrap(),
        1_767_398_399,
        "an inclusive date-only upper bound must end at 23:59:59 UTC"
    );
}
