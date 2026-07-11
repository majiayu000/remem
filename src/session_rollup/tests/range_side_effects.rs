use anyhow::Result;
use rusqlite::params;

use super::*;

fn transcript_message(text: &str) -> String {
    serde_json::json!({
        "type": "assistant",
        "message": {"content": [{"type": "text", "text": text}]}
    })
    .to_string()
}

fn transcript_role_message(role: &str, text: &str) -> String {
    serde_json::json!({
        "type": role,
        "message": {"content": [{"type": "text", "text": text}]}
    })
    .to_string()
}

#[tokio::test]
async fn session_rollup_prompt_includes_only_bounded_transcript_text() -> Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("session-rollup-prompt-transcript");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("transcript.jsonl");
    let before_user = transcript_role_message(
        "user",
        "transcript-only request: preserve the captured Stop boundary",
    );
    let before_assistant = transcript_role_message(
        "assistant",
        "Decision: keep SessionRollup prompts grounded in bounded transcript text.",
    );
    let after_stop = transcript_role_message(
        "assistant",
        "appended after Stop: this text must not enter the rollup prompt",
    );
    std::fs::write(&transcript, format!("{before_user}\n{before_assistant}\n"))?;
    let transcript_byte_len = std::fs::metadata(&transcript)?.len();

    let mut conn = setup_conn();
    let session_id = "sess-rollup-prompt-transcript";
    capture(
        &conn,
        session_id,
        "session_stop",
        &serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "transcript_path": transcript,
            "transcript_byte_len": transcript_byte_len
        })
        .to_string(),
    )?;
    std::fs::write(
        &transcript,
        format!("{before_user}\n{before_assistant}\n{after_stop}\n"),
    )?;
    let task = claim_rollup_task(&mut conn)?;

    let result = process_with_summarizer(&mut conn, &task, |prompt| async move {
        assert!(
            prompt.contains("transcript-only request: preserve the captured Stop boundary"),
            "bounded user transcript text missing from prompt: {prompt}"
        );
        assert!(
            prompt.contains(
                "Decision: keep SessionRollup prompts grounded in bounded transcript text."
            ),
            "bounded assistant transcript text missing from prompt: {prompt}"
        );
        assert!(!prompt.contains("appended after Stop"), "{prompt}");
        Ok(xml_response_with_structured_fields(
            "Use bounded transcript evidence.",
            "Preserve the captured Stop boundary.",
            "Decision: keep SessionRollup prompts grounded in bounded transcript text.",
            "",
            "",
            "",
            "",
        ))
    })
    .await?;

    assert_eq!(result, SessionRollupResult::Written);
    let candidate_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_candidates
         WHERE text LIKE '%Decision: keep SessionRollup prompts grounded in bounded transcript text.%'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(candidate_count, 1);
    Ok(())
}

#[tokio::test]
async fn session_rollup_missing_transcript_fails_before_metadata_only_summary() -> Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("session-rollup-prompt-missing");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("missing.jsonl");
    let mut conn = setup_conn();
    let session_id = "sess-rollup-prompt-missing";
    capture(
        &conn,
        session_id,
        "session_stop",
        &serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "transcript_path": transcript,
            "transcript_byte_len": 42
        })
        .to_string(),
    )?;
    let task = claim_rollup_task(&mut conn)?;

    let error = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("summarizer must not run without bounded transcript evidence")
    })
    .await
    .expect_err("missing transcript must keep the rollup retryable");

    assert!(
        error
            .to_string()
            .contains("read bounded transcript prompt evidence"),
        "{error:#}"
    );
    assert_eq!(summary_count(&conn), 0);
    Ok(())
}

#[tokio::test]
async fn session_rollup_unbounded_transcript_drains_archive_but_blocks_summary() -> Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("session-rollup-prompt-unbounded");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("legacy.jsonl");
    let transcript_text = "legacy transcript still belongs in the raw archive";
    std::fs::write(&transcript, transcript_message(transcript_text))?;
    let mut conn = setup_conn();
    let session_id = "sess-rollup-prompt-unbounded";
    capture(
        &conn,
        session_id,
        "session_stop",
        &serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "transcript_path": transcript
        })
        .to_string(),
    )?;
    let task = claim_rollup_task(&mut conn)?;

    let error = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("summarizer must not run without a captured transcript boundary")
    })
    .await
    .expect_err("unbounded transcript evidence must keep the rollup retryable");

    assert!(
        error.to_string().contains("transcript_byte_len"),
        "{error:#}"
    );
    assert_eq!(summary_count(&conn), 0);
    let archived: i64 = conn.query_row(
        "SELECT COUNT(*) FROM raw_messages
         WHERE session_id = ?1 AND content = ?2",
        params![session_id, transcript_text],
        |row| row.get(0),
    )?;
    assert_eq!(archived, 1);
    Ok(())
}

#[tokio::test]
async fn session_rollup_unusable_transcript_fails_before_metadata_only_summary() -> Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("session-rollup-prompt-unusable");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("unusable.jsonl");
    std::fs::write(
        &transcript,
        serde_json::json!({
            "type": "assistant",
            "message": {"content": []}
        })
        .to_string(),
    )?;
    let transcript_byte_len = std::fs::metadata(&transcript)?.len();
    let mut conn = setup_conn();
    let session_id = "sess-rollup-prompt-unusable";
    capture(
        &conn,
        session_id,
        "session_stop",
        &serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "transcript_path": transcript,
            "transcript_byte_len": transcript_byte_len
        })
        .to_string(),
    )?;
    let task = claim_rollup_task(&mut conn)?;

    let error = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("summarizer must not run without usable transcript evidence")
    })
    .await
    .expect_err("unusable transcript evidence must keep the rollup retryable");

    assert!(
        error
            .to_string()
            .contains("no usable user or assistant messages"),
        "{error:#}"
    );
    assert_eq!(summary_count(&conn), 0);
    Ok(())
}

#[tokio::test]
async fn session_rollup_candidate_evidence_stays_with_claimed_range() -> Result<()> {
    let mut conn = setup_conn();
    let session_id = "sess-rollup-candidate-range";
    let decision = "Keep candidate evidence bound to the rollup range that produced it.";
    capture(
        &conn,
        session_id,
        "session_stop",
        &serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "last_assistant_message": decision
        })
        .to_string(),
    )?;
    let first_event_id: i64 = conn.query_row(
        "SELECT MAX(id) FROM captured_events WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    let task = claim_rollup_task(&mut conn)?;

    capture(
        &conn,
        session_id,
        "session_stop",
        &serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "last_assistant_message": "This later Stop belongs to the next rollup range."
        })
        .to_string(),
    )?;
    let later_event_id: i64 = conn.query_row(
        "SELECT MAX(id) FROM captured_events WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    assert!(later_event_id > task.high_watermark_event_id.unwrap_or_default());

    let result = process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response_with_structured_fields(
            "Persist candidates from only the claimed range.",
            "Bind candidate evidence",
            decision,
            "",
            "Keep later captured events for the next rollup.",
            "",
            "",
        ))
    })
    .await?;
    assert_eq!(result, SessionRollupResult::Written);

    let evidence_json: String = conn.query_row(
        "SELECT evidence_event_ids
         FROM memory_candidates
         WHERE memory_type = 'decision'
           AND text LIKE ?1
         ORDER BY id DESC
         LIMIT 1",
        [format!("%{decision}%")],
        |row| row.get(0),
    )?;
    let evidence_event_ids: Vec<i64> = serde_json::from_str(&evidence_json)?;
    assert_eq!(evidence_event_ids, vec![first_event_id]);
    assert!(!evidence_event_ids.contains(&later_event_id));
    Ok(())
}

#[tokio::test]
async fn session_rollup_drains_every_coalesced_stop_payload() -> Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("session-rollup-coalesced-stops");
    std::fs::create_dir_all(&data_dir.path)?;
    let first_transcript = data_dir.path.join("first.jsonl");
    let second_transcript = data_dir.path.join("second.jsonl");
    std::fs::write(
        &first_transcript,
        transcript_message("first transcript stop"),
    )?;
    std::fs::write(
        &second_transcript,
        transcript_message("second transcript stop"),
    )?;
    let session_id = "sess-rollup-coalesced-stops";
    let mut conn = setup_conn();

    for payload in [
        serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "transcript_path": first_transcript,
            "transcript_byte_len": std::fs::metadata(&first_transcript)?.len()
        }),
        serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "last_assistant_message": "pathless fallback stop"
        }),
        serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "transcript_path": second_transcript,
            "transcript_byte_len": std::fs::metadata(&second_transcript)?.len()
        }),
    ] {
        capture(&conn, session_id, "session_stop", &payload.to_string())?;
    }
    let task = claim_rollup_task(&mut conn)?;

    let result = process_with_summarizer(&mut conn, &task, |prompt| async move {
        let first = prompt
            .find("first transcript stop")
            .expect("first transcript text should reach the prompt");
        let second = prompt
            .find("second transcript stop")
            .expect("second transcript text should reach the prompt");
        assert!(first < second, "{prompt}");
        Ok(xml_response("Drain all coalesced Stop payloads.", ""))
    })
    .await?;
    assert_eq!(result, SessionRollupResult::Written);

    let mut stmt = conn.prepare(
        "SELECT content, source
         FROM raw_messages
         WHERE session_id = ?1
         ORDER BY content ASC",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let archived = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        archived,
        vec![
            (
                "first transcript stop".to_string(),
                crate::memory::raw_archive::SOURCE_TRANSCRIPT.to_string(),
            ),
            (
                "pathless fallback stop".to_string(),
                crate::memory::raw_archive::SOURCE_HOOK.to_string(),
            ),
            (
                "second transcript stop".to_string(),
                crate::memory::raw_archive::SOURCE_TRANSCRIPT.to_string(),
            ),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn session_rollup_deduplicates_same_transcript_at_widest_stop_boundary() -> Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("session-rollup-same-transcript-stops");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("shared.jsonl");
    let first = transcript_message("first shared transcript stop");
    let second = transcript_message("second shared transcript stop");
    let after = transcript_message("after shared Stop boundary");
    std::fs::write(&transcript, format!("{first}\n"))?;
    let first_boundary = std::fs::metadata(&transcript)?.len();
    let session_id = "sess-rollup-same-transcript-stops";
    let mut conn = setup_conn();
    capture(
        &conn,
        session_id,
        "session_stop",
        &serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "transcript_path": transcript,
            "transcript_byte_len": first_boundary
        })
        .to_string(),
    )?;

    std::fs::write(&transcript, format!("{first}\n{second}\n"))?;
    let second_boundary = std::fs::metadata(&transcript)?.len();
    assert!(second_boundary > first_boundary);
    capture(
        &conn,
        session_id,
        "session_stop",
        &serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "transcript_path": transcript,
            "transcript_byte_len": second_boundary
        })
        .to_string(),
    )?;
    std::fs::write(&transcript, format!("{first}\n{second}\n{after}\n"))?;
    let task = claim_rollup_task(&mut conn)?;

    let result = process_with_summarizer(&mut conn, &task, |prompt| async move {
        assert_eq!(prompt.matches("first shared transcript stop").count(), 1);
        assert_eq!(prompt.matches("second shared transcript stop").count(), 1);
        assert!(!prompt.contains("after shared Stop boundary"));
        Ok(xml_response("Use the widest covered Stop boundary.", ""))
    })
    .await?;
    assert_eq!(result, SessionRollupResult::Written);

    let mut stmt = conn.prepare(
        "SELECT content
         FROM raw_messages
         WHERE session_id = ?1
         ORDER BY id ASC",
    )?;
    let archived = stmt
        .query_map([session_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        archived,
        vec![
            "first shared transcript stop".to_string(),
            "second shared transcript stop".to_string(),
        ]
    );
    assert!(!archived.contains(&"after shared Stop boundary".to_string()));
    Ok(())
}

#[tokio::test]
async fn session_rollup_prompt_does_not_duplicate_captured_message_text() -> Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("session-rollup-prompt-dedup");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("transcript.jsonl");
    let captured_text = "captured user request must appear once";
    let transcript_only_text = "transcript-only assistant outcome";
    std::fs::write(
        &transcript,
        format!(
            "{}\n{}\n",
            serde_json::json!({
                "type": "user",
                "message": {"content": [{"type": "text", "text": captured_text}]}
            }),
            transcript_message(transcript_only_text)
        ),
    )?;
    let boundary = std::fs::metadata(&transcript)?.len();
    let session_id = "sess-rollup-prompt-dedup";
    let mut conn = setup_conn();
    capture(&conn, session_id, "user_prompt_submit", captured_text)?;
    capture(
        &conn,
        session_id,
        "session_stop",
        &serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "transcript_path": transcript,
            "transcript_byte_len": boundary
        })
        .to_string(),
    )?;
    let task = claim_rollup_task(&mut conn)?;

    let result = process_with_summarizer(&mut conn, &task, |prompt| async move {
        assert_eq!(prompt.matches(captured_text).count(), 1, "{prompt}");
        assert_eq!(prompt.matches(transcript_only_text).count(), 1, "{prompt}");
        Ok(xml_response(
            "Deduplicate captured and transcript evidence.",
            "",
        ))
    })
    .await?;

    assert_eq!(result, SessionRollupResult::Written);
    Ok(())
}
