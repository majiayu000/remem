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
