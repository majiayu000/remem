use super::*;
use anyhow::Context;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    crate::migrate::run_migrations(&conn).expect("migrations should run");
    conn
}

#[test]
fn record_captured_event_coalesces_extraction_task_by_session() {
    let conn = setup_conn();
    let first = record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-1",
            project: "/tmp/remem",
            cwd: Some("/tmp/remem"),
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content: r#"{"session_id":"sess-1"}"#,
            task_kind: Some(ExtractionTaskKind::SessionRollup),
        },
    )
    .expect("first capture should insert");
    let second = record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-1",
            project: "/tmp/remem",
            cwd: Some("/tmp/remem"),
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: r#"{"tool_name":"Bash","command":"cargo test"}"#,
            task_kind: Some(ExtractionTaskKind::SessionRollup),
        },
    )
    .expect("second capture should insert");

    assert_eq!(first.extraction_task_id, second.extraction_task_id);
    let event_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))
        .unwrap();
    let task_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM extraction_tasks", [], |row| {
            row.get(0)
        })
        .unwrap();
    let high_watermark: i64 = conn
        .query_row(
            "SELECT high_watermark_event_id FROM extraction_tasks",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(event_count, 2);
    assert_eq!(task_count, 1);
    assert_eq!(high_watermark, second.event_row_id);
}

#[test]
fn large_capture_uses_blob_and_compact_preview() -> Result<()> {
    let conn = setup_conn();
    let content = "x".repeat(DIRECT_CONTENT_BYTES + 2048);
    let outcome = record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "claude-code",
            session_id: "sess-large",
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Task"),
            content: &content,
            task_kind: Some(ExtractionTaskKind::ObservationExtract),
        },
    )?;

    let (retention, blob_id, event_hash): (String, Option<i64>, String) = conn.query_row(
        "SELECT retention_class, content_blob_id, content_hash FROM captured_events WHERE id = ?1",
        params![outcome.event_row_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let blob_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM event_blobs", [], |row| row.get(0))?;
    let blob_hash: String =
        conn.query_row("SELECT content_hash FROM event_blobs", [], |row| row.get(0))?;
    assert_eq!(retention, "raw_compact");
    assert!(blob_id.is_some());
    assert_eq!(blob_count, 1);
    assert!(event_hash.starts_with("sha256:content-v1:"));
    assert!(blob_hash.starts_with("sha256:content-v1:"));
    Ok(())
}

#[test]
fn large_capture_reuses_matching_legacy_blob() -> Result<()> {
    let conn = setup_conn();
    let content = "legacy blob content ".repeat(1200);
    let sanitized_content = redact_capture_content(&content);
    let legacy_hash = legacy_exact_hash(&sanitized_content);
    conn.execute(
            "INSERT INTO event_blobs
             (content_hash, content_encoding, content_bytes, original_bytes, stored_bytes, created_at_epoch)
             VALUES (?1, 'plain', ?2, ?3, ?3, 100)",
            params![
                legacy_hash,
                sanitized_content.as_bytes(),
                sanitized_content.len() as i64
            ],
        )?;
    let legacy_blob_id = conn.last_insert_rowid();

    let outcome = record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "claude-code",
            session_id: "sess-legacy-blob",
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Task"),
            content: &content,
            task_kind: None,
        },
    )?;

    let (event_hash, blob_id): (String, Option<i64>) = conn.query_row(
        "SELECT content_hash, content_blob_id FROM captured_events WHERE id = ?1",
        params![outcome.event_row_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let blob_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM event_blobs", [], |row| row.get(0))?;
    assert!(event_hash.starts_with("sha256:content-v1:"));
    assert_eq!(blob_id, Some(legacy_blob_id));
    assert_eq!(blob_count, 1);
    Ok(())
}

#[test]
fn large_capture_does_not_reuse_mismatched_legacy_blob() -> Result<()> {
    let conn = setup_conn();
    let content = "target blob content ".repeat(1200);
    let sanitized_content = redact_capture_content(&content);
    let legacy_hash = legacy_exact_hash(&sanitized_content);
    let wrong_content = "different blob content".repeat(1200);
    conn.execute(
            "INSERT INTO event_blobs
             (content_hash, content_encoding, content_bytes, original_bytes, stored_bytes, created_at_epoch)
             VALUES (?1, 'plain', ?2, ?3, ?3, 100)",
            params![
                legacy_hash,
                wrong_content.as_bytes(),
                wrong_content.len() as i64
            ],
        )?;
    let wrong_blob_id = conn.last_insert_rowid();

    let outcome = record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "claude-code",
            session_id: "sess-mismatched-legacy-blob",
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Task"),
            content: &content,
            task_kind: None,
        },
    )?;

    let blob_id: i64 = conn.query_row(
        "SELECT content_blob_id FROM captured_events WHERE id = ?1",
        params![outcome.event_row_id],
        |row| row.get(0),
    )?;
    let blob_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM event_blobs", [], |row| row.get(0))?;
    assert_ne!(blob_id, wrong_blob_id);
    assert_eq!(blob_count, 2);
    Ok(())
}

#[test]
fn capture_redacts_sensitive_json_before_inline_storage() -> Result<()> {
    let conn = setup_conn();
    let content = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": {
                "command": "curl -H 'Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456' https://example.test",
                "api_key": "sk-secret-value"
            },
            "tool_response": {
                "stdout": "password=hunter2\nTOKEN=github_pat_secret"
            }
        })
        .to_string();
    let outcome = record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "claude-code",
            session_id: "sess-redact",
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: &content,
            task_kind: Some(ExtractionTaskKind::ObservationExtract),
        },
    )?;

    let stored: String = conn.query_row(
        "SELECT content_text FROM captured_events WHERE id = ?1",
        params![outcome.event_row_id],
        |row| row.get(0),
    )?;
    assert!(stored.contains("[REDACTED]"));
    assert!(!stored.contains("sk-secret-value"));
    assert!(!stored.contains("hunter2"));
    assert!(!stored.contains("github_pat_secret"));
    assert!(!stored.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
    Ok(())
}

#[test]
fn capture_redacts_sensitive_git_metadata_before_storage() -> Result<()> {
    let conn = setup_conn();
    let secret = "github_pat_abcdefghijklmnopqrstuvwxyz";
    let evidence = crate::git_util::GitCommitEvidence {
        kind: crate::git_util::GitEvidenceKind::ObservedCommit,
        metadata: crate::git_util::GitCommitMetadata {
            repo_path: "/tmp/remem".to_string(),
            sha: "abcdef1234567890abcdef1234567890abcdef12".to_string(),
            short_sha: "abcdef1".to_string(),
            branch: Some(format!("token={secret}")),
            message: Some(format!("avoid Authorization: Bearer {secret}")),
            authored_at_epoch: Some(1_700_000_000),
            changed_files: vec![format!("secrets/api_key={secret}")],
        },
        locator: Some("test".to_string()),
    };
    let outcome = record_captured_event_with_id_and_reference_time_and_git_evidence(
        &conn,
        &CaptureEventInput {
            host: "claude-code",
            session_id: "sess-git-redact",
            project: "/tmp/remem",
            cwd: Some("/tmp/remem"),
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: "{}",
            task_kind: Some(ExtractionTaskKind::ObservationExtract),
        },
        Some("git-redact"),
        Some(1_700_000_000),
        &[evidence],
    )?;

    let stored: String = conn.query_row(
        "SELECT metadata_json FROM captured_event_commits WHERE event_row_id = ?1",
        params![outcome.event_row_id],
        |row| row.get(0),
    )?;
    assert!(stored.contains("[REDACTED]"));
    assert!(!stored.contains(secret));
    Ok(())
}

#[test]
fn duplicate_capture_merges_git_evidence_recovered_later() -> Result<()> {
    let mut conn = setup_conn();
    let input = CaptureEventInput {
        host: "claude-code",
        session_id: "sess-late-git-evidence",
        project: "/tmp/remem",
        cwd: None,
        event_type: "tool_result",
        role: None,
        tool_name: Some("Bash"),
        content: "{}",
        task_kind: Some(ExtractionTaskKind::ObservationExtract),
    };
    let first = record_captured_event_with_id_and_reference_time_and_git_evidence(
        &conn,
        &input,
        Some("late-git-evidence"),
        None,
        &[],
    )?;
    let first_task_id = first
        .extraction_task_id
        .expect("initial capture should enqueue extraction");
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'done', cursor_event_id = high_watermark_event_id
         WHERE id = ?1",
        params![first_task_id],
    )?;
    let evidence = crate::git_util::GitCommitEvidence {
        kind: crate::git_util::GitEvidenceKind::ObservedCommit,
        metadata: crate::git_util::GitCommitMetadata {
            repo_path: "/tmp/remem".to_string(),
            sha: "abcdef1234567890abcdef1234567890abcdef12".to_string(),
            short_sha: "abcdef1".to_string(),
            branch: Some("main".to_string()),
            message: Some("commit".to_string()),
            authored_at_epoch: Some(1_700_000_000),
            changed_files: vec!["src/lib.rs".to_string()],
        },
        locator: Some("replayed_spill".to_string()),
    };
    let second = record_captured_event_with_id_and_reference_time_and_git_evidence(
        &conn,
        &input,
        Some("late-git-evidence"),
        None,
        std::slice::from_ref(&evidence),
    )?;
    record_captured_event_with_id_and_reference_time_and_git_evidence(
        &conn,
        &input,
        Some("late-git-evidence"),
        None,
        &[evidence],
    )?;

    assert_eq!(first.event_row_id, second.event_row_id);
    assert_ne!(second.extraction_task_id, Some(first_task_id));
    let (count, sha, task_count): (i64, String, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(MIN(sha), ''),
                (SELECT COUNT(*) FROM extraction_tasks)
         FROM captured_event_commits",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(count, 1);
    assert_eq!(sha, "abcdef1234567890abcdef1234567890abcdef12");
    assert_eq!(task_count, 2);
    let late_task = crate::db::claim_next_extraction_task(&mut conn, "worker", 60)?
        .context("late evidence should enqueue a bounded extraction task")?;
    assert_eq!(late_task.cursor_event_id, Some(first.event_row_id - 1));
    assert_eq!(late_task.high_watermark_event_id, Some(first.event_row_id));
    let linked = crate::captured_git::link_task_range(&mut conn, &late_task)?;
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].sha, sha);
    Ok(())
}

#[test]
fn capture_redaction_preserves_stop_payload_paths_for_worker_side_effects() -> Result<()> {
    let content = serde_json::json!({
            "cwd": "/Users/apple/.claude/projects/remem-abcdef1234567890abcdef1234567890",
            "transcript_path": "/Users/apple/.claude/projects/remem/session-abcdef1234567890abcdef1234567890.jsonl",
            "api_key": "sk-secret-value"
        })
        .to_string();

    let redacted: serde_json::Value = serde_json::from_str(&redact_capture_content(&content))?;

    assert_eq!(
        redacted["cwd"].as_str(),
        Some("/Users/apple/.claude/projects/remem-abcdef1234567890abcdef1234567890")
    );
    assert_eq!(
        redacted["transcript_path"].as_str(),
        Some("/Users/apple/.claude/projects/remem/session-abcdef1234567890abcdef1234567890.jsonl")
    );
    assert_eq!(redacted["api_key"].as_str(), Some("[REDACTED]"));
    Ok(())
}

#[test]
fn capture_redacts_sensitive_text_before_blob_storage() -> Result<()> {
    let conn = setup_conn();
    let mut content = "x".repeat(DIRECT_CONTENT_BYTES + 2048);
    content.push_str("\nAuthorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456\n");
    content.push_str("PASSWORD=hunter2\n");
    let outcome = record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "claude-code",
            session_id: "sess-redact-blob",
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: &content,
            task_kind: Some(ExtractionTaskKind::ObservationExtract),
        },
    )?;

    let stored: String = conn.query_row(
        "SELECT CAST(b.content_bytes AS TEXT)
                 FROM captured_events e
                 JOIN event_blobs b ON b.id = e.content_blob_id
                 WHERE e.id = ?1",
        params![outcome.event_row_id],
        |row| row.get(0),
    )?;
    assert!(stored.contains("[REDACTED]"));
    assert!(!stored.contains("hunter2"));
    assert!(!stored.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
    Ok(())
}

#[test]
fn capture_rejects_unknown_host() {
    let conn = setup_conn();
    let err = record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "unknown",
            session_id: "sess-host",
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Task"),
            content: "{}",
            task_kind: None,
        },
    )
    .expect_err("unknown host should fail closed");

    assert!(err.to_string().contains("invalid capture host"));
}
