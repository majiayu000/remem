use crate::adapter::{EventSummary, ParsedHookEvent};
use crate::db::{self, test_support::ScopedTestDataDir};

use super::super::spill::{spill_capture_event, SPILL_REASON_CAPTURE_PERSISTENCE_FAILED};
use super::observe_input;

#[tokio::test]
async fn observe_spills_projection_failure_and_replays_capture_once() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("observe-persist-failure-spill");
    let failing_input = serde_json::json!({
        "session_id": "sess-persist-fail",
        "cwd": "/tmp/remem",
        "tool_name": "Edit",
        "tool_input": {"file_path": "src/lib.rs"},
        "tool_response": {"content": "edited"}
    })
    .to_string();

    let conn = db::open_db()?;
    conn.execute_batch(
        "CREATE TRIGGER fail_events_insert
         BEFORE INSERT ON events
         BEGIN
             SELECT RAISE(FAIL, 'events blocked');
         END;",
    )?;
    drop(conn);

    let err = observe_input(&failing_input, Some("claude-code"))
        .await
        .expect_err("event insert failure should spill");
    assert!(err.to_string().contains("events blocked"), "{err}");
    assert!(crate::db::data_dir().join("capture-spill.jsonl").exists());

    let conn = db::open_db()?;
    let partial_captures: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events WHERE session_id = 'sess-persist-fail'",
        [],
        |row| row.get(0),
    )?;
    let partial_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = 'sess-persist-fail'",
        [],
        |row| row.get(0),
    )?;
    let partial_tasks: i64 = conn.query_row(
        "SELECT COUNT(*) FROM extraction_tasks WHERE session_row_id IN (
             SELECT id FROM sessions WHERE session_id = 'sess-persist-fail'
         )",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(partial_captures, 0);
    assert_eq!(partial_events, 0);
    assert_eq!(partial_tasks, 0);
    let partial_drop: (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COUNT(recovered_event_id)
         FROM capture_drop_events
         WHERE session_id = 'sess-persist-fail'
           AND reason = 'capture_persistence_failed'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let partial_stats = db::query_system_stats(&conn)?;
    assert_eq!(partial_drop, (1, 0));
    assert_eq!(partial_stats.actionable_capture_drops, 1);
    assert_eq!(partial_stats.unrecovered_capture_spills, 1);
    conn.execute_batch("DROP TRIGGER fail_events_insert;")?;
    drop(conn);

    let replay_trigger = serde_json::json!({
        "session_id": "sess-replay-trigger",
        "cwd": "/tmp/remem",
        "tool_name": "Edit",
        "tool_input": {"file_path": "src/other.rs"},
        "tool_response": {"content": "edited"}
    })
    .to_string();
    observe_input(&replay_trigger, Some("claude-code")).await?;

    let conn = db::open_db()?;
    let replayed_captures: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events WHERE session_id = 'sess-persist-fail'",
        [],
        |row| row.get(0),
    )?;
    let replayed_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = 'sess-persist-fail'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(replayed_captures, 1);
    assert_eq!(replayed_events, 1);
    let linked_event_id: i64 = conn.query_row(
        "SELECT captured_event_id FROM events WHERE session_id = 'sess-persist-fail'",
        [],
        |row| row.get(0),
    )?;
    assert!(linked_event_id > 0);
    let replayed_drop: (String, Option<i64>) = conn.query_row(
        "SELECT reason, recovered_event_id
         FROM capture_drop_events
         WHERE session_id = 'sess-persist-fail'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let replayed_stats = db::query_system_stats(&conn)?;
    assert_eq!(replayed_drop.0, SPILL_REASON_CAPTURE_PERSISTENCE_FAILED);
    assert!(replayed_drop.1.is_some());
    assert_eq!(replayed_stats.actionable_capture_drops, 0);
    assert_eq!(replayed_stats.unrecovered_capture_spills, 0);
    assert!(!crate::db::data_dir().join("capture-spill.jsonl").exists());

    let replayed_event_id: String = conn.query_row(
        "SELECT event_id FROM captured_events WHERE session_id = 'sess-persist-fail'",
        [],
        |row| row.get(0),
    )?;
    let replayed_summary = conn.query_row(
        "SELECT event_type, summary, detail, files, exit_code
         FROM events
         WHERE session_id = 'sess-persist-fail'",
        [],
        |row| {
            Ok(EventSummary {
                event_type: row.get(0)?,
                summary: row.get(1)?,
                detail: row.get(2)?,
                files_json: row.get(3)?,
                exit_code: row.get(4)?,
            })
        },
    )?;
    drop(conn);

    let replayed_event = ParsedHookEvent {
        session_id: "sess-persist-fail".to_string(),
        cwd: Some("/tmp/remem".to_string()),
        project: "/tmp/remem".to_string(),
        reference_time_epoch: None,
        tool_name: "Edit".to_string(),
        tool_input: Some(serde_json::json!({"file_path": "src/lib.rs"})),
        tool_response: Some(serde_json::json!({"content": "edited"})),
    };
    spill_capture_event(
        "claude-code",
        &replayed_event_id,
        &replayed_event,
        &replayed_summary,
        SPILL_REASON_CAPTURE_PERSISTENCE_FAILED,
        &anyhow::anyhow!("retry same partial capture"),
    )?;
    observe_input(&replay_trigger, Some("claude-code")).await?;

    let conn = db::open_db()?;
    let retry_replayed_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = 'sess-persist-fail'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(retry_replayed_events, 1);
    Ok(())
}

#[test]
fn dsh_assistant_capture_requires_corroboration_for_summary_promotion() -> anyhow::Result<()> {
    let cwd = ScopedTestDataDir::new_offline("dsh-assistant-provenance");
    std::fs::create_dir_all(&cwd.path)?;
    let decision = "Use source kind telemetry for summary promotion gate";
    for (tool_host, tool_name, tool_content) in [
        ("deepseek-harness", "fixture_probe", None),
        (
            "deepseek-harness",
            "fixture_probe",
            Some("cargo test passed"),
        ),
        ("deepseek-harness", "browser_search", Some(decision)),
        ("deepseek-harness", "Read", Some(decision)),
        ("claude-code", "Bash", Some(decision)),
    ] {
        let mut conn = rusqlite::Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let mut project = String::new();
        let mut assistant_id = 0;
        let mut event_ids = Vec::new();
        let mut source_texts = Vec::new();
        // Exercise the real DSH adapter and durable observe writer. Repeating an
        // assistant claim, even beside an unrelated tool result, is not support.
        for (index, (host, tool_name, content)) in [
            Some(("deepseek-harness", "assistant/message", decision)),
            Some(("deepseek-harness", "assistant/message", decision)),
            tool_content.map(|content| (tool_host, tool_name, content)),
        ]
        .into_iter()
        .flatten()
        .enumerate()
        {
            let payload = serde_json::json!({
                "host": host, "session_id": "dsh-provenance",
                "cwd": cwd.path, "tool_name": tool_name,
                "tool_input": {"command": "cargo test"},
                "tool_response": {"text": content, "exitCode": 0}
            });
            let (adapter, event) = super::detect_adapter_for_host(&payload.to_string(), Some(host))
                .expect("DSH input should be recognized");
            let summary = adapter.classify_event(&event).unwrap();
            project = event.project.clone();
            let id = super::record_observed_event_with_id(
                &conn,
                adapter.name(),
                &format!("dsh-{index}"),
                &event,
                &summary,
                &[],
            )?;
            event_ids.push(id);
            source_texts.push(conn.query_row(
                "SELECT content_text FROM captured_events WHERE id = ?1",
                [id],
                |row| row.get(0),
            )?);
            if index == 0 {
                assistant_id = id;
            }
        }
        let project_id = conn.query_row(
            "SELECT project_id FROM captured_events WHERE id = ?1",
            [assistant_id],
            |row| row.get(0),
        )?;
        assert_eq!(
            crate::memory::promote::promote_summary_to_memory_candidates_with_evidence(
                &mut conn,
                "dsh-provenance",
                project_id,
                &project,
                &event_ids,
                &source_texts,
                None,
                Some(decision),
                None,
                None,
            )?,
            1
        );
        let (status, trust, reason, memory_count): (String, String, Option<String>, i64) = conn
            .query_row(
                "SELECT review_status, source_trust_class, auto_promote_block_reason,
                    (SELECT COUNT(*) FROM memories) FROM memory_candidates",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        if tool_host == "claude-code" {
            assert_eq!(
                (status.as_str(), trust.as_str(), reason, memory_count),
                ("auto_promoted", "local_tool_output", None, 1)
            );
        } else {
            assert_eq!(
                status, "pending_review",
                "assistant-only claim auto-promoted; tool={tool_content:?}"
            );
            assert_eq!(trust, "external_content");
            assert_eq!(reason.as_deref(), Some("source_trust_below_floor"));
            assert_eq!(memory_count, 0);
        }
        let (kind, role, tool, content): (String, Option<String>, Option<String>, String) = conn
            .query_row(
            "SELECT event_type, role, tool_name, content_text FROM captured_events WHERE id = ?1",
            [assistant_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(kind, "message");
        assert_eq!(role.as_deref(), Some("assistant"));
        assert_eq!(tool, None);
        assert!(content.contains(decision));
    }
    Ok(())
}
