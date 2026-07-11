use crate::adapter::{EventSummary, ParsedHookEvent};
use crate::db::{self, test_support::ScopedTestDataDir};

use super::{replay_spilled_capture_events, spill_capture_event, SPILL_REASON_DB_OPEN_FAILED};

#[test]
fn replay_spilled_capture_event_records_capture_and_drop_ledger() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("capture-spill-replay");
    let err = anyhow::anyhow!("database is locked");
    let event = ParsedHookEvent {
        session_id: "session-spill".to_string(),
        cwd: Some("/tmp/remem".to_string()),
        project: "/tmp/remem".to_string(),
        reference_time_epoch: None,
        tool_name: "Edit".to_string(),
        tool_input: Some(serde_json::json!({"file_path": "/tmp/remem/src/lib.rs"})),
        tool_response: Some(serde_json::json!({"ok": true})),
    };
    let summary = EventSummary {
        event_type: "file_edit".to_string(),
        summary: "Edited src/lib.rs".to_string(),
        detail: None,
        files_json: Some("[\"src/lib.rs\"]".to_string()),
        exit_code: None,
    };
    let content = serde_json::json!({
        "summary": summary.summary,
        "tool_name": event.tool_name,
        "tool_input": event.tool_input,
        "tool_response": event.tool_response,
    })
    .to_string();
    let event_id = db::unique_capture_event_id("tool_result", &content);
    spill_capture_event(
        "codex-cli",
        &event_id,
        &event,
        &summary,
        SPILL_REASON_DB_OPEN_FAILED,
        &err,
    )?;
    let conn = db::open_db()?;

    let replayed = replay_spilled_capture_events(&conn)?;

    assert_eq!(replayed, 1);
    let captured: i64 =
        conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
    let drops: i64 = conn.query_row("SELECT COUNT(*) FROM capture_drop_events", [], |row| {
        row.get(0)
    })?;
    let drop_reason: String =
        conn.query_row("SELECT reason FROM capture_drop_events", [], |row| {
            row.get(0)
        })?;
    assert_eq!(captured, 1);
    assert_eq!(drops, 1);
    assert_eq!(drop_reason, SPILL_REASON_DB_OPEN_FAILED);
    Ok(())
}

#[test]
fn replay_identical_spills_preserves_distinct_captured_events() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("capture-spill-identical-replay");
    let err = anyhow::anyhow!("database is locked");
    let event = ParsedHookEvent {
        session_id: "session-identical-spill".to_string(),
        cwd: Some("/tmp/remem".to_string()),
        project: "/tmp/remem".to_string(),
        reference_time_epoch: None,
        tool_name: "Edit".to_string(),
        tool_input: Some(serde_json::json!({"file_path": "/tmp/remem/src/lib.rs"})),
        tool_response: Some(serde_json::json!({"ok": true})),
    };
    let summary = EventSummary {
        event_type: "file_edit".to_string(),
        summary: "Edited src/lib.rs".to_string(),
        detail: None,
        files_json: Some("[\"src/lib.rs\"]".to_string()),
        exit_code: None,
    };

    spill_capture_event(
        "codex-cli",
        "tool_result-identical-a",
        &event,
        &summary,
        SPILL_REASON_DB_OPEN_FAILED,
        &err,
    )?;
    spill_capture_event(
        "codex-cli",
        "tool_result-identical-b",
        &event,
        &summary,
        SPILL_REASON_DB_OPEN_FAILED,
        &err,
    )?;
    let conn = db::open_db()?;

    let replayed = replay_spilled_capture_events(&conn)?;

    assert_eq!(replayed, 2);
    let captured: i64 =
        conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
    let events: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
    assert_eq!(captured, 2);
    assert_eq!(events, 2);
    Ok(())
}

#[test]
fn replay_identical_spill_retry_appends_partial_second_legacy_event() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("capture-spill-identical-partial-retry");
    let err = anyhow::anyhow!("database is locked");
    let event = ParsedHookEvent {
        session_id: "session-identical-partial".to_string(),
        cwd: Some("/tmp/remem".to_string()),
        project: "/tmp/remem".to_string(),
        reference_time_epoch: None,
        tool_name: "Edit".to_string(),
        tool_input: Some(serde_json::json!({"file_path": "/tmp/remem/src/lib.rs"})),
        tool_response: Some(serde_json::json!({"ok": true})),
    };
    let summary = EventSummary {
        event_type: "file_edit".to_string(),
        summary: "Edited src/lib.rs".to_string(),
        detail: None,
        files_json: Some("[\"src/lib.rs\"]".to_string()),
        exit_code: None,
    };
    spill_capture_event(
        "codex-cli",
        "tool_result-identical-partial-a",
        &event,
        &summary,
        SPILL_REASON_DB_OPEN_FAILED,
        &err,
    )?;
    spill_capture_event(
        "codex-cli",
        "tool_result-identical-partial-b",
        &event,
        &summary,
        SPILL_REASON_DB_OPEN_FAILED,
        &err,
    )?;
    let conn = db::open_db()?;
    conn.execute_batch(
        "CREATE TRIGGER fail_second_legacy_event
             BEFORE INSERT ON events
             WHEN (SELECT COUNT(*) FROM events WHERE session_id = 'session-identical-partial') >= 1
             BEGIN
                 SELECT RAISE(FAIL, 'legacy events blocked');
             END;",
    )?;

    assert_eq!(replay_spilled_capture_events(&conn)?, 1);
    let partial_captured: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events WHERE session_id = 'session-identical-partial'",
        [],
        |row| row.get(0),
    )?;
    let partial_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = 'session-identical-partial'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(partial_captured, 1);
    assert_eq!(partial_events, 1);

    conn.execute_batch("DROP TRIGGER fail_second_legacy_event;")?;
    assert_eq!(replay_spilled_capture_events(&conn)?, 1);
    let replayed_captured: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events WHERE session_id = 'session-identical-partial'",
        [],
        |row| row.get(0),
    )?;
    let replayed_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = 'session-identical-partial'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(replayed_captured, 2);
    assert_eq!(replayed_events, 2);
    Ok(())
}

#[test]
fn replay_legacy_spill_without_event_id() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("capture-spill-legacy-replay");
    let legacy = serde_json::json!({
        "version": 1,
        "host": "codex-cli",
        "event": {
            "session_id": "session-legacy-spill",
            "cwd": "/tmp/remem",
            "project": "/tmp/remem",
            "tool_name": "Edit",
            "tool_input": {"file_path": "/tmp/remem/src/lib.rs"},
            "tool_response": {"ok": true}
        },
        "summary": {
            "event_type": "file_edit",
            "summary": "Edited src/lib.rs with ghp_abcdefghijklmnopqrstuvwxyz123456",
            "detail": "password=hunter2",
            "files_json": "[\"src/lib.rs\"]",
            "exit_code": null
        },
        "db_error": "database token=github_pat_secret",
        "created_at_epoch": 1700000000
    });
    let path = crate::db::data_dir().join("capture-spill.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, format!("{legacy}\n"))?;
    let conn = db::open_db()?;

    let replayed = replay_spilled_capture_events(&conn)?;

    assert_eq!(replayed, 1);
    let captured: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events WHERE session_id = 'session-legacy-spill'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(captured, 1);
    let replayed_event: (String, String) = conn.query_row(
        "SELECT summary, detail FROM events WHERE session_id = 'session-legacy-spill'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let drop_detail: String = conn.query_row(
        "SELECT detail FROM capture_drop_events WHERE session_id = 'session-legacy-spill'",
        [],
        |row| row.get(0),
    )?;
    assert!(replayed_event.0.contains("[REDACTED]"));
    assert!(!replayed_event
        .0
        .contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
    assert!(!replayed_event.1.contains("hunter2"));
    assert!(!drop_detail.contains("github_pat_secret"));
    assert!(!path.exists());
    Ok(())
}

#[test]
fn replay_legacy_spill_preserves_synthesized_id_after_partial_failure() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("capture-spill-legacy-stable-id");
    let legacy = serde_json::json!({
        "version": 1,
        "host": "codex-cli",
        "event": {
            "session_id": "session-legacy-stable",
            "cwd": "/tmp/remem",
            "project": "/tmp/remem",
            "tool_name": "Edit",
            "tool_input": {"file_path": "/tmp/remem/src/lib.rs"},
            "tool_response": {"ok": true}
        },
        "summary": {
            "event_type": "file_edit",
            "summary": "Edited src/lib.rs",
            "detail": null,
            "files_json": "[\"src/lib.rs\"]",
            "exit_code": null
        },
        "db_error": "database is locked",
        "created_at_epoch": 1700000000
    });
    let path = crate::db::data_dir().join("capture-spill.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, format!("{legacy}\n"))?;
    let conn = db::open_db()?;
    conn.execute_batch(
        "CREATE TRIGGER fail_legacy_events_insert
             BEFORE INSERT ON events
             BEGIN
                 SELECT RAISE(FAIL, 'legacy events blocked');
             END;",
    )?;

    let replayed = replay_spilled_capture_events(&conn)?;

    assert_eq!(replayed, 0);
    let partial_captures: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events WHERE session_id = 'session-legacy-stable'",
        [],
        |row| row.get(0),
    )?;
    let partial_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = 'session-legacy-stable'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(partial_captures, 0);
    assert_eq!(partial_events, 0);
    let failed_spill = std::fs::read_to_string(&path)?;
    assert!(failed_spill.contains(r#""event_id":"tool_result-legacy-spill-"#));

    conn.execute_batch("DROP TRIGGER fail_legacy_events_insert;")?;
    let replayed = replay_spilled_capture_events(&conn)?;

    assert_eq!(replayed, 1);
    let replayed_captures: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events WHERE session_id = 'session-legacy-stable'",
        [],
        |row| row.get(0),
    )?;
    let replayed_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = 'session-legacy-stable'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(replayed_captures, 1);
    assert_eq!(replayed_events, 1);
    assert!(!path.exists());
    Ok(())
}

#[test]
fn spill_redacts_summary_and_database_error() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("capture-spill-redacts");
    let err = anyhow::anyhow!("database token=github_pat_secret");
    let event = ParsedHookEvent {
        session_id: "session-spill-redact".to_string(),
        cwd: Some("/tmp/remem".to_string()),
        project: "/tmp/remem".to_string(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({
            "command": "curl -H 'Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456'"
        })),
        tool_response: Some(serde_json::json!({"stderr": "password=hunter2"})),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "Run curl with ghp_abcdefghijklmnopqrstuvwxyz123456".to_string(),
        detail: Some("password=hunter2".to_string()),
        files_json: None,
        exit_code: Some(1),
    };

    spill_capture_event(
        "codex-cli",
        "tool_result-redact",
        &event,
        &summary,
        SPILL_REASON_DB_OPEN_FAILED,
        &err,
    )?;
    let stored = std::fs::read_to_string(crate::db::data_dir().join("capture-spill.jsonl"))?;

    assert!(stored.contains("[REDACTED]"));
    assert!(!stored.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
    assert!(!stored.contains("hunter2"));
    assert!(!stored.contains("github_pat_secret"));
    Ok(())
}

#[test]
fn encrypted_capture_spill_hides_payload_and_replays() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("capture-spill-encrypted");
    std::env::set_var("REMEM_CIPHER_KEY", format!("v2:{}", "2".repeat(64)));
    let err = anyhow::anyhow!("database is locked");
    let event = ParsedHookEvent {
        session_id: "session-encrypted-spill".to_string(),
        cwd: Some("/tmp/remem".to_string()),
        project: "/tmp/remem".to_string(),
        reference_time_epoch: None,
        tool_name: "Write".to_string(),
        tool_input: Some(serde_json::json!({"content": "ordinary source content"})),
        tool_response: Some(serde_json::json!({"ok": true})),
    };
    let summary = EventSummary {
        event_type: "file_write".to_string(),
        summary: "Wrote ordinary source content".to_string(),
        detail: Some("ordinary source content".to_string()),
        files_json: None,
        exit_code: None,
    };

    spill_capture_event(
        "codex-cli",
        "tool_result-encrypted-spill",
        &event,
        &summary,
        SPILL_REASON_DB_OPEN_FAILED,
        &err,
    )?;
    let stored = std::fs::read_to_string(crate::db::data_dir().join("capture-spill.jsonl"))?;
    assert!(stored.contains("remem-spill-v1"));
    assert!(!stored.contains("ordinary source content"));

    let conn = db::open_db()?;
    assert_eq!(replay_spilled_capture_events(&conn)?, 1);
    let replayed_summary: String = conn.query_row(
        "SELECT summary FROM events WHERE session_id = 'session-encrypted-spill'",
        [],
        |row| row.get(0),
    )?;
    assert!(replayed_summary.contains("ordinary source content"));
    Ok(())
}
