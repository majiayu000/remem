use anyhow::Result;

use super::*;

fn capture_codex_transcript_message(
    conn: &rusqlite::Connection,
    session_id: &str,
    role: &str,
    content: &str,
) -> Result<i64> {
    let outcome = db::record_captured_event(
        conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: "/tmp/remem",
            cwd: None,
            event_type: "message",
            role: Some(role),
            tool_name: Some("codex-transcript"),
            content,
            task_kind: Some(db::ExtractionTaskKind::SessionRollup),
        },
    )?;
    Ok(outcome.event_row_id)
}

#[tokio::test]
async fn session_rollup_promotes_summary_claim_from_codex_message_event() -> Result<()> {
    let mut conn = setup_conn();
    let session_id = "sess-rollup-message-evidence";
    let request = "Fix summary evidence binding";
    let decision = "Transcript messages are captured as immutable evidence for summary promotion.";
    let candidate_text = format!("[Context: {request}]\n\n{decision}");
    let message_id =
        capture_codex_transcript_message(&conn, session_id, "assistant", &candidate_text)?;
    capture(
        &conn,
        session_id,
        "session_stop",
        &serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem"
        })
        .to_string(),
    )?;
    let stop_id: i64 = conn.query_row(
        "SELECT id FROM captured_events
         WHERE session_id = ?1 AND event_type = 'session_stop'",
        [session_id],
        |row| row.get(0),
    )?;
    let task = claim_rollup_task(&mut conn)?;

    let result = process_with_summarizer(&mut conn, &task, |_prompt| async move {
        Ok(xml_response_with_structured_fields(
            "Promote summary claims from captured Codex message evidence.",
            request,
            decision,
            "",
            "",
            "",
            "",
        ))
    })
    .await?;
    assert_eq!(result, SessionRollupResult::Written);

    let (review_status, evidence_json, source_trust): (String, String, String) = conn.query_row(
        "SELECT review_status, evidence_event_ids, source_trust_class
         FROM memory_candidates
         WHERE text = ?1",
        [&candidate_text],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(review_status, "auto_promoted");
    assert_eq!(
        serde_json::from_str::<Vec<i64>>(&evidence_json)?,
        vec![message_id]
    );
    assert!(!serde_json::from_str::<Vec<i64>>(&evidence_json)?.contains(&stop_id));
    assert_eq!(source_trust, "local_tool_output");

    let memory_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE content = ?1",
        [&candidate_text],
        |row| row.get(0),
    )?;
    assert_eq!(memory_count, 1);
    Ok(())
}
