use super::*;

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
