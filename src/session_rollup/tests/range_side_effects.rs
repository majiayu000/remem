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

    let result = process_with_summarizer(&mut conn, &task, |_prompt| async {
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

    let result = process_with_summarizer(&mut conn, &task, |_prompt| async {
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
