use crate::db::{self, test_support::ScopedTestDataDir};

use super::{replay_spilled_summary_hook_payloads, spill_summary_hook_payload, summary_spill_path};

#[tokio::test]
async fn stale_db_spill_replays_after_schema_is_initialized() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-hook-spill-replay");
    std::fs::create_dir_all(&test_dir.path)?;
    let setup = rusqlite::Connection::open(test_dir.db_path())?;
    setup.execute("CREATE TABLE marker (id INTEGER PRIMARY KEY)", [])?;
    drop(setup);
    let spilled_input = serde_json::json!({
        "session_id": "sess-summary-spilled",
        "cwd": "/tmp/remem"
    })
    .to_string();

    let err = super::super::hook::summarize_input(&spilled_input, Some("codex-cli"), None)
        .await
        .expect_err("stale hook database should spill and fail closed");

    assert!(
        err.to_string().contains("hook database open requires"),
        "unexpected error: {err:#}"
    );
    assert!(summary_spill_path().exists());
    std::fs::remove_file(test_dir.db_path())?;

    let conn = db::open_db()?;
    let now = chrono::Utc::now().timestamp();
    db::upsert_worker_heartbeat(
        &conn,
        "worker-daemon",
        i64::from(std::process::id()),
        now,
        now,
    )?;
    drop(conn);

    let current_input = serde_json::json!({
        "session_id": "sess-summary-current",
        "cwd": "/tmp/remem"
    })
    .to_string();
    super::super::hook::summarize_input(&current_input, Some("codex-cli"), None).await?;

    assert!(!summary_spill_path().exists());
    let conn = db::open_db()?;
    let rollup_tasks: i64 = conn.query_row(
        "SELECT COUNT(*) FROM extraction_tasks t
             JOIN sessions s ON s.id = t.session_row_id
             WHERE t.task_kind = 'session_rollup'
               AND s.session_id IN ('sess-summary-spilled', 'sess-summary-current')",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(rollup_tasks, 2);
    Ok(())
}

#[tokio::test]
async fn current_stop_payload_wins_over_same_session_spill_replay() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-hook-current-wins");
    let conn = db::open_db()?;
    let now = chrono::Utc::now().timestamp();
    db::upsert_worker_heartbeat(
        &conn,
        &db::current_worker_owner("daemon", std::process::id(), now * 1000),
        i64::from(std::process::id()),
        now,
        now,
    )?;
    let old_transcript = test_dir.path.join("old-transcript.jsonl");
    let current_transcript = test_dir.path.join("current-transcript.jsonl");
    std::fs::write(&old_transcript, "old transcript\n")?;
    std::fs::write(&current_transcript, "current transcript\n")?;
    let old_input = serde_json::json!({
        "session_id": "sess-summary-current-wins",
        "cwd": "/tmp/remem",
        "transcript_path": old_transcript
    })
    .to_string();
    spill_summary_hook_payload(
        &old_input,
        Some("codex-cli"),
        None,
        Some("/tmp/remem"),
        &anyhow::anyhow!("stale db"),
    )?;

    let current_input = serde_json::json!({
        "session_id": "sess-summary-current-wins",
        "cwd": "/tmp/remem",
        "transcript_path": current_transcript
    })
    .to_string();
    super::super::hook::summarize_input(&current_input, Some("codex-cli"), None).await?;

    let (event_count, payload): (i64, String) = conn.query_row(
        "SELECT COUNT(*), COALESCE(MAX(content_text), '')
             FROM captured_events
             WHERE event_type = 'session_stop'
               AND session_id = 'sess-summary-current-wins'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(event_count, 1);
    assert!(payload.contains(current_transcript.to_string_lossy().as_ref()));
    assert!(!payload.contains(old_transcript.to_string_lossy().as_ref()));
    Ok(())
}

#[test]
fn replay_preserves_spills_appended_after_claim() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-spill-claim-race");
    let conn = db::open_db()?;
    let first_input = serde_json::json!({
        "session_id": "sess-summary-first",
        "cwd": "/tmp/remem"
    })
    .to_string();
    let later_input = serde_json::json!({
        "session_id": "sess-summary-later",
        "cwd": "/tmp/remem"
    })
    .to_string();
    spill_summary_hook_payload(
        &first_input,
        Some("codex-cli"),
        None,
        Some("/tmp/remem"),
        &anyhow::anyhow!("stale db"),
    )?;

    let mut wrote_later = false;
    let replayed = replay_spilled_summary_hook_payloads(&conn, |_conn, record| {
        assert_eq!(record.input, first_input);
        if !wrote_later {
            spill_summary_hook_payload(
                &later_input,
                Some("codex-cli"),
                None,
                Some("/tmp/remem"),
                &anyhow::anyhow!("still stale"),
            )?;
            wrote_later = true;
        }
        Ok(())
    })?;

    assert_eq!(replayed, 1);
    let remaining = std::fs::read_to_string(summary_spill_path())?;
    assert!(remaining.contains("sess-summary-later"));
    assert!(!remaining.contains("sess-summary-first"));
    Ok(())
}

#[test]
fn spill_payload_fills_cwd_and_protects_last_assistant_message() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-spill-sanitize");
    std::env::set_var("REMEM_CIPHER_KEY", format!("v2:{}", "1".repeat(64)));
    let input = serde_json::json!({
        "session_id": "sess-summary-sensitive",
        "last_assistant_message": "private assistant answer"
    })
    .to_string();

    spill_summary_hook_payload(
        &input,
        Some("codex-cli"),
        Some("quality"),
        Some("/tmp/original-project"),
        &anyhow::anyhow!("stale db"),
    )?;

    let stored = std::fs::read_to_string(summary_spill_path())?;
    assert!(!stored.contains("private assistant answer"));
    let record: super::SummaryHookSpillRecord =
        crate::db::spill_crypto::decode_json_line(stored.trim())?;
    let payload: serde_json::Value = serde_json::from_str(&record.input)?;
    assert_eq!(payload["cwd"].as_str(), Some("/tmp/original-project"));
    assert_eq!(payload["remem_ai_profile"].as_str(), Some("quality"));
    assert_eq!(
        payload["last_assistant_message"].as_str(),
        Some("private assistant answer")
    );
    Ok(())
}

#[test]
fn restore_claimed_spill_makes_records_visible_to_future_replay() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-spill-restore-claim");
    std::fs::create_dir_all(db::data_dir())?;
    let claimed_path = db::data_dir().join("summary-hook-spill.replay-test.jsonl");
    let failed_path = super::failed_summary_spill_path_for_claim(&claimed_path);
    std::fs::write(
        &claimed_path,
        format!(
            "{}\n",
            r#"{"version":1,"input":"{\"session_id\":\"sess-restored\"}","host":"codex-cli","profile":null,"db_error":"stale","created_at_epoch":1}"#
        ),
    )?;
    std::fs::write(
        &failed_path,
        format!(
            "{}\n",
            r#"{"version":1,"input":"{\"session_id\":\"sess-duplicate\"}","host":"codex-cli","profile":null,"db_error":"stale","created_at_epoch":1}"#
        ),
    )?;
    std::fs::write(
        summary_spill_path(),
        format!(
            "{}\n",
            r#"{"version":1,"input":"{\"session_id\":\"sess-active\"}","host":"codex-cli","profile":null,"db_error":"stale","created_at_epoch":1}"#
        ),
    )?;

    super::restore_claimed_and_failed_spill(&claimed_path, &failed_path, &summary_spill_path());

    assert!(!claimed_path.exists());
    assert!(!failed_path.exists());
    let restored = std::fs::read_to_string(summary_spill_path())?;
    assert!(restored.contains("sess-active"));
    assert!(restored.contains("sess-restored"));
    assert!(!restored.contains("sess-duplicate"));
    Ok(())
}

#[test]
fn orphaned_claimed_spill_is_restored_to_active_queue() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-spill-orphan-claim");
    std::fs::create_dir_all(db::data_dir())?;
    let claimed_path = db::data_dir().join("summary-hook-spill.replay-orphan.jsonl");
    let failed_path = super::failed_summary_spill_path_for_claim(&claimed_path);
    std::fs::write(
        &claimed_path,
        format!(
            "{}\n",
            r#"{"version":1,"input":"{\"session_id\":\"sess-orphan\"}","host":"codex-cli","profile":null,"db_error":"stale","created_at_epoch":1}"#
        ),
    )?;
    std::fs::write(
        &failed_path,
        format!(
            "{}\n",
            r#"{"version":1,"input":"{\"session_id\":\"sess-orphan-failed\"}","host":"codex-cli","profile":null,"db_error":"stale","created_at_epoch":1}"#
        ),
    )?;

    let restored = super::restore_orphaned_summary_spill_claims(std::time::Duration::ZERO)?;

    assert_eq!(restored, 1);
    assert!(!claimed_path.exists());
    assert!(!failed_path.exists());
    let active = std::fs::read_to_string(summary_spill_path())?;
    assert!(active.contains("sess-orphan"));
    assert!(!active.contains("sess-orphan-failed"));
    Ok(())
}

#[test]
fn fresh_claimed_spill_is_not_restored_as_orphan() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-spill-fresh-claim");
    std::fs::create_dir_all(db::data_dir())?;
    let claimed_path = super::claimed_summary_spill_path();
    std::fs::write(
        &claimed_path,
        format!(
            "{}\n",
            r#"{"version":1,"input":"{\"session_id\":\"sess-fresh-claim\"}","host":"codex-cli","profile":null,"db_error":"stale","created_at_epoch":1}"#
        ),
    )?;

    let restored =
        super::restore_orphaned_summary_spill_claims(std::time::Duration::from_secs(60))?;

    assert_eq!(restored, 0);
    assert!(claimed_path.exists());
    assert!(!summary_spill_path().exists());
    Ok(())
}

#[test]
fn live_claimed_spill_is_not_restored_as_orphan() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-spill-live-claim");
    std::fs::create_dir_all(db::data_dir())?;
    let claimed_path = db::data_dir().join(format!(
        "summary-hook-spill.replay-{}-1-0.jsonl",
        std::process::id()
    ));
    std::fs::write(
        &claimed_path,
        format!(
            "{}\n",
            r#"{"version":1,"input":"{\"session_id\":\"sess-live-claim\"}","host":"codex-cli","profile":null,"db_error":"stale","created_at_epoch":1}"#
        ),
    )?;

    let restored =
        super::restore_orphaned_summary_spill_claims(std::time::Duration::from_secs(60))?;

    assert_eq!(restored, 0);
    assert!(claimed_path.exists());
    assert!(!summary_spill_path().exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn stale_dead_claimed_spill_is_restored_as_orphan() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-spill-dead-claim");
    std::fs::create_dir_all(db::data_dir())?;
    let claimed_path = db::data_dir().join(format!(
        "summary-hook-spill.replay-{}-1-0.jsonl",
        i64::from(i32::MAX)
    ));
    std::fs::write(
        &claimed_path,
        format!(
            "{}\n",
            r#"{"version":1,"input":"{\"session_id\":\"sess-dead-claim\"}","host":"codex-cli","profile":null,"db_error":"stale","created_at_epoch":1}"#
        ),
    )?;

    let restored =
        super::restore_orphaned_summary_spill_claims(std::time::Duration::from_secs(60))?;

    assert_eq!(restored, 1);
    assert!(!claimed_path.exists());
    let active = std::fs::read_to_string(summary_spill_path())?;
    assert!(active.contains("sess-dead-claim"));
    Ok(())
}

#[tokio::test]
async fn replay_error_does_not_drop_current_summary_payload() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-replay-error-current");
    let conn = db::open_db()?;
    let now = chrono::Utc::now().timestamp();
    db::upsert_worker_heartbeat(
        &conn,
        "worker-daemon",
        i64::from(std::process::id()),
        now,
        now,
    )?;
    drop(conn);
    std::fs::create_dir_all(summary_spill_path())?;

    let current_input = serde_json::json!({
        "session_id": "sess-summary-current-after-replay-error",
        "cwd": "/tmp/remem"
    })
    .to_string();
    super::super::hook::summarize_input(&current_input, Some("codex-cli"), None).await?;

    let conn = db::open_db()?;
    let rollup_tasks: i64 = conn.query_row(
        "SELECT COUNT(*) FROM extraction_tasks t
             JOIN sessions s ON s.id = t.session_row_id
             WHERE t.task_kind = 'session_rollup'
               AND s.session_id = 'sess-summary-current-after-replay-error'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(rollup_tasks, 1);
    Ok(())
}
