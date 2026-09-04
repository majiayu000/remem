use std::sync::Mutex;

use crate::db::{self, test_support::ScopedTestDataDir};

use super::{
    record_replayed_git_evidence_only, record_summary_capture_event, resolve_hook_host,
    summarize_input, summary_payload_with_cwd,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    old_values: Vec<(String, Option<String>)>,
}

impl EnvGuard {
    fn set(vars: &[(&str, Option<&str>)]) -> Self {
        let old_values = vars
            .iter()
            .map(|(key, _)| ((*key).to_string(), std::env::var(key).ok()))
            .collect::<Vec<_>>();

        for (key, value) in vars {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }

        Self { old_values }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.old_values.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(&key, value) },
                None => unsafe { std::env::remove_var(&key) },
            }
        }
    }
}

fn with_env_vars<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    let Ok(_guard) = ENV_LOCK.lock() else {
        panic!("env lock should acquire");
    };
    let _env = EnvGuard::set(vars);
    f()
}

#[test]
fn hook_host_normalizes_explicit_host() {
    with_env_vars(
        &[
            ("REMEM_SUMMARY_EXECUTOR", Some("claude-cli")),
            ("REMEM_EXECUTOR", Some("claude-cli")),
        ],
        || {
            assert!(matches!(
                resolve_hook_host(Some("codex")).as_deref(),
                Ok("codex-cli")
            ));
        },
    );
}

#[test]
fn hook_host_uses_runtime_config_default() {
    let _test_dir = ScopedTestDataDir::new("summary-default-host");

    with_env_vars(
        &[
            ("REMEM_HOOK_HOST", None),
            ("REMEM_CONTEXT_HOST", None),
            ("REMEM_SUMMARY_EXECUTOR", None),
            ("REMEM_EXECUTOR", None),
        ],
        || {
            assert!(matches!(
                resolve_hook_host(None).as_deref(),
                Ok("codex-cli")
            ));
        },
    );
}

#[test]
fn hook_host_preserves_legacy_summary_executor() {
    with_env_vars(
        &[
            ("REMEM_HOOK_HOST", None),
            ("REMEM_CONTEXT_HOST", None),
            ("REMEM_SUMMARY_EXECUTOR", Some("claude-cli")),
            ("REMEM_EXECUTOR", Some("codex-cli")),
        ],
        || {
            assert!(matches!(
                resolve_hook_host(None).as_deref(),
                Ok("claude-code")
            ));
        },
    );
}

#[test]
fn summary_payload_with_cwd_fills_missing_cwd() {
    let payload = summary_payload_with_cwd(r#"{"session_id":"sess-cwd"}"#, "/tmp/project", None)
        .expect("payload should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&payload).expect("payload should parse");

    assert_eq!(parsed["session_id"].as_str(), Some("sess-cwd"));
    assert_eq!(parsed["cwd"].as_str(), Some("/tmp/project"));
}

#[test]
fn summary_payload_with_cwd_preserves_existing_cwd() {
    let payload = summary_payload_with_cwd(
        r#"{"session_id":"sess-cwd","cwd":"/repo"}"#,
        "/tmp/project",
        None,
    )
    .expect("payload should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&payload).expect("payload should parse");

    assert_eq!(parsed["cwd"].as_str(), Some("/repo"));
}

#[test]
fn summary_payload_preserves_profile() {
    let summary = summary_payload_with_cwd(
        r#"{"session_id":"sess-cwd"}"#,
        "/tmp/project",
        Some("custom"),
    )
    .expect("payload should serialize");
    let summary: serde_json::Value =
        serde_json::from_str(&summary).expect("summary payload should parse");

    assert_eq!(summary["remem_ai_profile"].as_str(), Some("custom"));
}

#[test]
fn summary_payload_snapshots_transcript_byte_length() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-transcript-byte-length");
    std::fs::create_dir_all(&test_dir.path)?;
    let transcript = test_dir.path.join("transcript.jsonl");
    std::fs::write(&transcript, "first line\nsecond line\n")?;
    let input = serde_json::json!({
        "session_id": "sess-transcript-byte-length",
        "transcript_path": transcript
    })
    .to_string();

    let payload = summary_payload_with_cwd(&input, "/tmp/project", None)?;
    let parsed: serde_json::Value = serde_json::from_str(&payload)?;

    assert_eq!(
        parsed["transcript_byte_len"].as_u64(),
        Some(std::fs::metadata(&transcript)?.len())
    );
    Ok(())
}

#[test]
fn hosted_summary_hook_can_preserve_profile_override() -> anyhow::Result<()> {
    let host = resolve_hook_host(Some("codex"))?;
    let summary = summary_payload_with_cwd(
        r#"{"session_id":"sess-hosted-profile"}"#,
        "/tmp/project",
        Some("custom"),
    )?;
    let parsed: serde_json::Value = serde_json::from_str(&summary)?;

    assert_eq!(host, "codex-cli");
    assert_eq!(parsed["remem_ai_profile"].as_str(), Some("custom"));
    Ok(())
}

#[tokio::test]
async fn summarize_hook_rejects_stale_schema_without_migrating() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-hook-stale-schema");
    std::fs::create_dir_all(&test_dir.path)?;
    let setup = rusqlite::Connection::open(test_dir.db_path())?;
    setup.execute("CREATE TABLE marker (id INTEGER PRIMARY KEY)", [])?;
    drop(setup);
    let input = serde_json::json!({
        "session_id": "sess-summary-stale",
        "cwd": "/tmp/remem"
    })
    .to_string();

    let err = summarize_input(&input, Some("codex-cli"), None)
        .await
        .expect_err("stale hook database should fail closed");

    assert!(
        err.to_string().contains("hook database open requires"),
        "unexpected error: {err:#}"
    );
    let check = rusqlite::Connection::open(test_dir.db_path())?;
    let (migrations_exists, jobs_exists): (i64, i64) = check.query_row(
        "SELECT
                SUM(CASE WHEN name = '_schema_migrations' THEN 1 ELSE 0 END),
                SUM(CASE WHEN name = 'jobs' THEN 1 ELSE 0 END)
             FROM sqlite_master
             WHERE type = 'table'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(migrations_exists, 0);
    assert_eq!(jobs_exists, 0);
    assert!(super::super::spill::summary_spill_path().exists());
    Ok(())
}

#[tokio::test]
async fn summarize_hook_spills_when_capture_ledger_fails() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-capture-failure");
    drop(db::open_db()?);
    match std::fs::remove_file(super::super::spill::summary_spill_path()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let input = serde_json::json!({
        "session_id": "sess-summary-capture-failure",
        "cwd": "/tmp/remem"
    })
    .to_string();

    let err = summarize_input(&input, Some("unknown"), None)
        .await
        .expect_err("capture ledger failure should fail closed");

    assert!(
        err.to_string().contains("invalid capture host"),
        "unexpected error: {err:#}"
    );
    let conn = db::open_db()?;
    let job_count: i64 = conn.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))?;
    assert_eq!(job_count, 0);
    assert!(super::super::spill::summary_spill_path().exists());
    Ok(())
}

#[tokio::test]
async fn summarize_hook_spills_when_transcript_snapshot_fails() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-hook-transcript-snapshot-failure");
    drop(db::open_db()?);
    let missing_transcript = test_dir.path.join("missing-transcript.jsonl");
    let input = serde_json::json!({
        "session_id": "sess-summary-transcript-snapshot-failure",
        "cwd": "/tmp/remem",
        "transcript_path": missing_transcript
    })
    .to_string();

    let result = summarize_input(&input, Some("codex-cli"), None).await;
    let error = match result {
        Ok(()) => anyhow::bail!("missing transcript snapshot unexpectedly succeeded"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("snapshot transcript length"));
    let conn = db::open_db()?;
    let captured_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events
             WHERE session_id = 'sess-summary-transcript-snapshot-failure'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(captured_events, 0);
    assert!(super::super::spill::summary_spill_path().exists());
    Ok(())
}

#[tokio::test]
async fn unresolvable_commit_evidence_does_not_drop_stop_capture() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-hook-unresolvable-git-evidence");
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
    let missing_repo = test_dir.path.join("missing-repo");
    let transcript = test_dir.path.join("rollout.jsonl");
    let call = serde_json::json!({
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "name": "exec_command",
            "call_id": "commit-call",
            "arguments": serde_json::json!({
                "cmd": "git commit -m done",
                "workdir": missing_repo,
            }).to_string(),
        }
    });
    let output = serde_json::json!({
        "type": "response_item",
        "payload": {
            "type": "function_call_output",
            "call_id": "commit-call",
            "output": "Process exited with code 0\nFinal output:\n[main deadbeef] done",
        }
    });
    std::fs::write(&transcript, format!("{call}\n{output}\n"))?;
    let input = serde_json::json!({
        "session_id": "sess-stop-unresolvable-git-evidence",
        "cwd": test_dir.path,
        "transcript_path": transcript,
        "last_assistant_message": "finished"
    })
    .to_string();

    summarize_input(&input, Some("codex-cli"), None).await?;

    let conn = db::open_db()?;
    let captured: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events
         WHERE session_id = 'sess-stop-unresolvable-git-evidence'
           AND event_type = 'session_stop'",
        [],
        |row| row.get(0),
    )?;
    let evidence: i64 =
        conn.query_row("SELECT COUNT(*) FROM captured_event_commits", [], |row| {
            row.get(0)
        })?;
    assert_eq!(captured, 1);
    assert_eq!(evidence, 0);
    Ok(())
}

#[tokio::test]
async fn codex_stop_capture_materializes_transcript_messages_before_session_stop(
) -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-hook-codex-message-events");
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

    let transcript = test_dir.path.join("rollout.jsonl");
    let user = serde_json::json!({
        "timestamp": "2026-06-12T00:00:01Z",
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Use the bounded transcript for evidence."}]
        }
    });
    let reasoning = serde_json::json!({
        "timestamp": "2026-06-12T00:00:02Z",
        "type": "response_item",
        "payload": {"type": "reasoning", "summary": []}
    });
    let meta_user = serde_json::json!({
        "timestamp": "2026-06-12T00:00:02Z",
        "type": "user",
        "isMeta": true,
        "message": {"content": "Host control metadata must not become user evidence."}
    });
    let xml_control_user = serde_json::json!({
        "timestamp": "2026-06-12T00:00:02Z",
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "<system>host control</system>"}]
        }
    });
    let assistant = serde_json::json!({
        "timestamp": "2026-06-12T00:00:03Z",
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "Decision: materialize Codex transcript messages as captured evidence."}]
        }
    });
    std::fs::write(
        &transcript,
        format!("{user}\n{meta_user}\n{xml_control_user}\n{reasoning}\n{assistant}\n"),
    )?;
    let input = serde_json::json!({
        "session_id": "sess-codex-message-events",
        "cwd": test_dir.path,
        "transcript_path": transcript
    })
    .to_string();

    summarize_input(&input, Some("codex-cli"), None).await?;

    let conn = db::open_db()?;
    let mut stmt = conn.prepare(
        "SELECT id, event_type, role, tool_name, content_text, created_at_epoch
         FROM captured_events
         WHERE session_id = 'sess-codex-message-events'
         ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].1, "message");
    assert_eq!(rows[0].2.as_deref(), Some("user"));
    assert_eq!(rows[0].3.as_deref(), Some("codex-transcript"));
    assert_eq!(rows[0].4, "Use the bounded transcript for evidence.");
    assert_eq!(rows[0].5, 1_781_222_401);
    assert_eq!(rows[1].1, "message");
    assert_eq!(rows[1].2.as_deref(), Some("assistant"));
    assert_eq!(rows[1].3.as_deref(), Some("codex-transcript"));
    assert_eq!(
        rows[1].4,
        "Decision: materialize Codex transcript messages as captured evidence."
    );
    assert_eq!(rows[1].5, 1_781_222_403);
    assert_eq!(rows[2].1, "session_stop");

    assert_eq!(
        crate::memory::poisoning::derive_source_trust_class(&conn, &[rows[0].0], "summary")?
            .as_str(),
        "user_prompt"
    );
    assert_eq!(
        crate::memory::poisoning::derive_source_trust_class(&conn, &[rows[1].0], "summary")?
            .as_str(),
        "external_content"
    );
    Ok(())
}

#[tokio::test]
async fn codex_stop_deduplicates_only_the_exact_captured_prompt_identity() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-hook-codex-prompt-dedup");
    std::fs::create_dir_all(&test_dir.path)?;
    let session_id = "sess-codex-prompt-dedup";
    let turn_id = "turn-already-rolled-up";
    let repeated_prompt = "Keep repeated prompts distinct by turn.";
    let project = db::project_from_cwd(&test_dir.path.to_string_lossy());
    let conn = db::open_db()?;
    let event_id = crate::identity::EventId::synthesize(
        Some(&crate::identity::TurnId(turn_id.to_string())),
        "UserPromptSubmit",
        None,
    )
    .0;
    let captured = db::record_captured_event_with_id_and_turn_id(
        &conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: &project,
            cwd: Some(&project),
            event_type: "user_prompt_submit",
            role: Some("user"),
            tool_name: None,
            content: repeated_prompt,
            task_kind: Some(db::ExtractionTaskKind::SessionRollup),
        },
        Some(&event_id),
        Some(turn_id),
    )?;
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'done', cursor_event_id = high_watermark_event_id
         WHERE id = ?1",
        [captured.extraction_task_id.expect("rollup task")],
    )?;
    let now = chrono::Utc::now().timestamp();
    let worker_owner = db::current_worker_owner("daemon", std::process::id(), now * 1_000);
    db::upsert_worker_heartbeat(
        &conn,
        &worker_owner,
        i64::from(std::process::id()),
        now,
        now,
    )?;
    drop(conn);

    let transcript = test_dir.path.join("rollout.jsonl");
    let message = |role: &str, text: &str, message_turn_id: &str, timestamp: &str| {
        serde_json::json!({
            "timestamp": timestamp,
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": role,
                "content": [{
                    "type": if role == "user" { "input_text" } else { "output_text" },
                    "text": text
                }],
                "internal_chat_message_metadata_passthrough": {
                    "turn_id": message_turn_id
                }
            }
        })
    };
    let exact_mirror = message("user", repeated_prompt, turn_id, "2026-06-12T00:00:01Z");
    let same_content_other_turn = message(
        "user",
        repeated_prompt,
        "turn-distinct",
        "2026-06-12T00:00:02Z",
    );
    let same_turn_other_content = message(
        "user",
        "Different content in the same turn remains transcript evidence.",
        turn_id,
        "2026-06-12T00:00:03Z",
    );
    let assistant = message(
        "assistant",
        "Assistant transcript evidence remains available.",
        turn_id,
        "2026-06-12T00:00:04Z",
    );
    std::fs::write(
        &transcript,
        format!(
            "{exact_mirror}\n{same_content_other_turn}\n{same_turn_other_content}\n{assistant}\n"
        ),
    )?;
    let input = serde_json::json!({
        "session_id": session_id,
        "cwd": project,
        "transcript_path": transcript
    })
    .to_string();

    summarize_input(&input, Some("codex-cli"), None).await?;

    let conn = db::open_db()?;
    let (cursor, high_watermark): (i64, i64) = conn.query_row(
        "SELECT cursor_event_id, high_watermark_event_id
         FROM extraction_tasks
         WHERE id = ?1",
        [captured.extraction_task_id.expect("rollup task")],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(cursor, captured.event_row_id);
    assert!(
        high_watermark > cursor,
        "expected Stop events after advanced cursor; cursor={cursor} high_watermark={high_watermark}"
    );

    let mut stmt = conn.prepare(
        "SELECT role, content_text
         FROM captured_events
         WHERE session_id = ?1
           AND id > ?2
           AND event_type = 'message'
           AND tool_name = 'codex-transcript'
         ORDER BY id ASC",
    )?;
    let later_messages = stmt
        .query_map(rusqlite::params![session_id, cursor], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        later_messages,
        vec![
            (Some("user".to_string()), repeated_prompt.to_string()),
            (
                Some("user".to_string()),
                "Different content in the same turn remains transcript evidence.".to_string(),
            ),
            (
                Some("assistant".to_string()),
                "Assistant transcript evidence remains available.".to_string(),
            ),
        ]
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM captured_events
             WHERE session_id = ?1
               AND event_id = ?2
               AND event_type = 'user_prompt_submit'",
            rusqlite::params![session_id, event_id],
            |row| row.get::<_, i64>(0),
        )?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn codex_stop_deduplicates_fixture_prompt_without_turn_id() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-hook-codex-fixture-dedup");
    std::fs::create_dir_all(&test_dir.path)?;
    let session_id = "sess-codex-fixture-dedup";
    let repeated_prompt = "Codex rollout user text should enter the raw archive.";
    let project = db::project_from_cwd(&test_dir.path.to_string_lossy());
    let conn = db::open_db()?;
    let captured = db::record_captured_event_with_id_and_turn_id(
        &conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: &project,
            cwd: Some(&project),
            event_type: "user_prompt_submit",
            role: Some("user"),
            tool_name: None,
            content: repeated_prompt,
            task_kind: Some(db::ExtractionTaskKind::SessionRollup),
        },
        Some(&crate::db::unique_capture_event_id(
            "user_prompt_submit",
            repeated_prompt,
        )),
        None,
    )?;
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'done', cursor_event_id = high_watermark_event_id
         WHERE id = ?1",
        [captured.extraction_task_id.expect("rollup task")],
    )?;
    let now = chrono::Utc::now().timestamp();
    let worker_owner = db::current_worker_owner("daemon", std::process::id(), now * 1_000);
    db::upsert_worker_heartbeat(
        &conn,
        &worker_owner,
        i64::from(std::process::id()),
        now,
        now,
    )?;
    drop(conn);

    let transcript = test_dir.path.join("rollout.jsonl");
    std::fs::write(
        &transcript,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/codex-rollout-minimal.jsonl"
        )),
    )?;
    let input = serde_json::json!({
        "session_id": session_id,
        "cwd": project,
        "transcript_path": transcript
    })
    .to_string();

    summarize_input(&input, Some("codex-cli"), None).await?;

    let conn = db::open_db()?;
    let user_messages: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events
         WHERE session_id = ?1
           AND event_type = 'message'
           AND role = 'user'
           AND content_text = ?2",
        rusqlite::params![session_id, repeated_prompt],
        |row| row.get(0),
    )?;
    assert_eq!(
        user_messages, 0,
        "fixture user turns without turn_id must not recapture an existing prompt"
    );
    let assistant_messages: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events
         WHERE session_id = ?1
           AND event_type = 'message'
           AND role = 'assistant'",
        rusqlite::params![session_id],
        |row| row.get(0),
    )?;
    assert!(
        assistant_messages > 0,
        "assistant fixture turns must still be captured"
    );
    Ok(())
}

#[tokio::test]
async fn summarize_hook_runs_stop_side_effects_without_summary_job() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-side-effects");
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
    let input = serde_json::json!({
        "session_id": "sess-summary-hook-side-effects",
        "cwd": "/tmp/remem",
        "last_assistant_message": "hook side effect assistant message"
    })
    .to_string();

    summarize_input(&input, Some("codex-cli"), None).await?;

    let conn = db::open_db()?;
    let captured_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events
             WHERE session_id = 'sess-summary-hook-side-effects'
               AND event_type = 'session_stop'",
        [],
        |row| row.get(0),
    )?;
    let job_count: i64 = conn.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))?;
    let summary_jobs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE job_type = 'summary'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(captured_events, 1);
    assert_eq!(job_count, 0);
    assert_eq!(summary_jobs, 0);
    Ok(())
}

#[tokio::test]
async fn summarize_hook_replays_same_session_spill_for_different_project() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-hook-project-scoped-spill");
    let conn = db::open_db()?;
    let now = chrono::Utc::now().timestamp();
    db::upsert_worker_heartbeat(
        &conn,
        &db::current_worker_owner("daemon", std::process::id(), now * 1000),
        i64::from(std::process::id()),
        now,
        now,
    )?;
    let old_transcript = test_dir.path.join("old-other-transcript.jsonl");
    let current_transcript = test_dir.path.join("current-transcript.jsonl");
    std::fs::write(
        &old_transcript,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"old"}]}}"#,
    )?;
    std::fs::write(
        &current_transcript,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"current"}]}}"#,
    )?;
    let old_input = serde_json::json!({
        "session_id": "sess-summary-shared-id",
        "cwd": "/tmp/remem-other",
        "transcript_path": old_transcript
    })
    .to_string();
    super::super::spill::spill_summary_hook_payload(
        &old_input,
        Some("codex-cli"),
        None,
        Some("/tmp/remem-other"),
        &anyhow::anyhow!("stale db"),
    )?;

    let current_input = serde_json::json!({
        "session_id": "sess-summary-shared-id",
        "cwd": "/tmp/remem-current",
        "transcript_path": current_transcript
    })
    .to_string();
    summarize_input(&current_input, Some("codex-cli"), None).await?;

    let event_count: i64 = conn.query_row(
        "SELECT COUNT(*)
             FROM captured_events
             WHERE event_type = 'session_stop'
               AND session_id = 'sess-summary-shared-id'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(event_count, 2);
    assert!(!super::super::spill::summary_spill_path().exists());
    Ok(())
}

#[test]
fn replayed_same_identity_git_evidence_is_idempotent_and_link_only() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-replayed-git-evidence-link-only");
    let mut conn = db::open_db()?;
    let sha = "abcdef1234567890abcdef1234567890abcdef12";
    let record: super::super::spill::SummaryHookSpillRecord =
        serde_json::from_value(serde_json::json!({
            "version": 2,
            "input": serde_json::json!({
                "session_id": "sess-replayed-git-evidence-link-only",
                "cwd": "/tmp/remem"
            }).to_string(),
            "host": "codex-cli",
            "profile": null,
            "git_evidence": [{
                "kind": "observed_commit",
                "metadata": {
                    "repo_path": "/tmp/remem",
                    "sha": sha,
                    "short_sha": "abcdef1",
                    "branch": "main",
                    "message": "commit",
                    "authored_at_epoch": 1_700_000_000,
                    "changed_files": ["src/lib.rs"]
                },
                "locator": "replayed_spill"
            }],
            "db_error": "database unavailable",
            "created_at_epoch": 1_700_000_000
        }))?;

    record_replayed_git_evidence_only(&conn, &record)?;
    record_replayed_git_evidence_only(&conn, &record)?;

    let event_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events
         WHERE session_id = 'sess-replayed-git-evidence-link-only'
           AND event_type = 'commit_evidence'",
        [],
        |row| row.get(0),
    )?;
    let task_kinds: Vec<String> = {
        let mut statement = conn.prepare("SELECT task_kind FROM extraction_tasks ORDER BY id")?;
        let rows = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        rows
    };
    assert_eq!(event_count, 1);
    assert_eq!(task_kinds, vec!["captured_git_link"]);

    let task = db::claim_next_extraction_task(&mut conn, "worker-replayed-link", 60)?
        .expect("replayed evidence should enqueue link-only work");
    assert_eq!(task.task_kind, db::ExtractionTaskKind::CapturedGitLink);
    let linked = crate::captured_git::link_task_range(&mut conn, &task)?;
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].sha, sha);
    Ok(())
}

#[test]
fn capture_ledger_failure_blocks_followup_jobs() {
    let _test_dir = ScopedTestDataDir::new("summary-legacy-unknown-host");
    let conn = db::open_db().expect("db should open");

    let err = record_summary_capture_event(
        &conn,
        "unknown",
        "sess-legacy",
        "/tmp/remem",
        "/tmp/remem",
        r#"{"session_id":"sess-legacy","cwd":"/tmp/remem"}"#,
        None,
        &[],
        None,
    )
    .expect_err("capture ledger failure should stop summary hook followups");

    assert!(err.to_string().contains("invalid capture host"));
    let job_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("job count should query");
    assert_eq!(job_count, 0);
}
