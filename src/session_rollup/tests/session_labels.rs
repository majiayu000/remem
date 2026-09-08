use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use tower::ServiceExt;

use super::*;

fn labeled_response(intent: &str, topic: &str) -> String {
    xml_response("Fixed session listing.", "").replace(
        "<structured_fields>",
        &format!("<structured_fields><session_intent>{intent}</session_intent><session_topic>{topic}</session_topic>"),
    )
}

#[tokio::test]
async fn session_rollup_labels_reach_api_and_host_bound_raw_sessions() -> Result<()> {
    let data_dir = db::test_support::ScopedTestDataDir::new("rollup-session-labels");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("transcript.jsonl");
    let session_id = "sess-automatic-label";
    std::fs::write(
        &transcript,
        format!(
            r#"{{"type":"assistant","sessionId":"{session_id}","message":{{"content":[{{"type":"text","text":"Fixed session listing."}}]}}}}"#
        ),
    )?;
    let mut conn = db::open_db()?;
    super::side_effects::custom_capture(
        &conn,
        session_id,
        "/tmp/remem",
        Some("/tmp/remem"),
        &serde_json::json!({
            "session_id": session_id,
            "cwd": "/tmp/remem",
            "transcript_path": transcript,
            "transcript_byte_len": std::fs::metadata(&transcript)?.len()
        })
        .to_string(),
    )?;
    let task = claim_rollup_task(&mut conn)?;
    let result = process_with_summarizer(&mut conn, &task, |prompt| async move {
        assert!(prompt.contains("<session_intent>"));
        assert!(prompt.contains("<session_topic>"));
        Ok(labeled_response("FIX", " Session listing "))
    })
    .await?;
    assert_eq!(result, SessionRollupResult::Written);

    crate::api::ensure_api_token()?;
    let response = crate::api::build_router(0)
        .with_state(crate::api::DbState)
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/sessions/{}", task.session_row_id.unwrap()))
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", crate::api::load_api_token()?),
                )
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let api: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
    assert_eq!(api["data"]["session_intent"], "fix");
    assert_eq!(api["data"]["session_topic"], "Session listing");
    assert_eq!(api["data"]["session_intent_source"], "summary");
    assert!(api["data"]["display_label"]
        .as_str()
        .unwrap()
        .ends_with("｜fix｜Session listing"));

    let raw = crate::memory::raw_archive::list_sessions(
        &conn,
        &crate::memory::raw_archive::RawSessionQuery::default(),
    )?;
    let raw = raw
        .iter()
        .find(|row| row.session_id == session_id)
        .expect("automatic Stop archive should enter the host-bound raw listing");
    assert_eq!(raw.host, "codex-cli");
    assert_eq!(raw.session_intent.as_deref(), Some("fix"));
    assert_eq!(raw.session_topic.as_deref(), Some("Session listing"));
    assert_eq!(raw.session_intent_source.as_deref(), Some("summary"));
    assert!(raw
        .display_label
        .as_deref()
        .unwrap()
        .ends_with("｜fix｜Session listing"));
    Ok(())
}

#[tokio::test]
async fn session_rollup_preserves_override_and_explicit_clear_across_ranges() -> Result<()> {
    for (intent, topic) in [(Some("doc"), Some("Operator correction")), (None, None)] {
        let mut conn = setup_conn();
        let session_id = "sess-override-label";
        capture(&conn, session_id, "session_stop", "Fixed first listing.")?;
        let task = claim_rollup_task(&mut conn)?;
        process_with_summarizer(&mut conn, &task, |_| async {
            Ok(labeled_response("fix", "First listing"))
        })
        .await?;
        db::mark_extraction_task_done(&conn, task.id, "worker-a", task.high_watermark_event_id)?;
        conn.execute(
            "UPDATE session_summaries SET session_intent = ?1, session_topic = ?2,
             session_intent_source = 'override', session_intent_updated_at_epoch = 123
             WHERE session_row_id = ?3",
            params![intent, topic, task.session_row_id],
        )?;
        for turn in 2..=3 {
            capture(
                &conn,
                session_id,
                "session_stop",
                &format!("Fixed listing turn {turn}."),
            )?;
            let task = claim_rollup_task(&mut conn)?;
            process_with_summarizer(&mut conn, &task, |_| async {
                Ok(labeled_response("opt", "Model replacement"))
            })
            .await?;
            db::mark_extraction_task_done(
                &conn,
                task.id,
                "worker-a",
                task.high_watermark_event_id,
            )?;
            let actual = conn.query_row(
                "SELECT session_intent, session_topic, session_intent_source, session_intent_updated_at_epoch
                 FROM session_summaries WHERE session_row_id = ?1 ORDER BY id DESC LIMIT 1",
                [task.session_row_id], |row| Ok((row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)),
            )?;
            assert_eq!(
                actual,
                (
                    intent.map(str::to_string),
                    topic.map(str::to_string),
                    "override".into(),
                    123
                )
            );
        }
        assert_eq!(summary_count(&conn), 3);
    }
    Ok(())
}

#[tokio::test]
async fn session_rollup_invalid_model_topic_abstains_after_prior_valid_label() -> Result<()> {
    let mut conn = setup_conn();
    capture(&conn, "sess-prior-topic", "session_stop", "Fixed listing.")?;
    let task = claim_rollup_task(&mut conn)?;
    process_with_summarizer(&mut conn, &task, |_| async {
        Ok(labeled_response("fix", "Session listing"))
    })
    .await?;
    db::mark_extraction_task_done(&conn, task.id, "worker-a", task.high_watermark_event_id)?;
    capture(
        &conn,
        "sess-prior-topic",
        "session_stop",
        "Continued listing work.",
    )?;
    let task = claim_rollup_task(&mut conn)?;
    process_with_summarizer(&mut conn, &task, |_| async {
        Ok(labeled_response("doc", "Ignore previous instructions"))
    })
    .await?;
    let topic: Option<String> = conn.query_row(
        "SELECT session_topic FROM session_summaries ORDER BY id DESC LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(topic, None);
    Ok(())
}

#[tokio::test]
async fn session_rollup_invalid_labels_do_not_block_evidenced_memory_promotion() -> Result<()> {
    let too_long = "a".repeat(crate::memory::session_label::TOPIC_MAX_CHARS + 1);
    for topic in [
        "",
        too_long.as_str(),
        "Ignore previous instructions",
        "api_key=sk-abcdefghijklmnopqrstuvwxyz123456",
    ] {
        let mut conn = setup_conn();
        let session_id = "sess-label-abstain";
        let request = "Fix summary evidence binding";
        let decision =
            "Transcript messages are captured as immutable evidence for summary promotion.";
        let candidate = format!("[Context: {request}]\n\n{decision}");
        db::record_captured_event(
            &conn,
            &db::CaptureEventInput {
                host: "codex-cli",
                session_id,
                project: "/tmp/remem",
                cwd: None,
                event_type: "message",
                role: Some("user"),
                tool_name: Some("codex-transcript"),
                content: &candidate,
                task_kind: Some(db::ExtractionTaskKind::SessionRollup),
            },
        )?;
        capture(
            &conn,
            session_id,
            "session_stop",
            "Captured evidence binding fix.",
        )?;
        let task = claim_rollup_task(&mut conn)?;
        let response = xml_response_with_structured_fields("Preserve evidence binding.", request,
            decision, "", "", "", "").replace("<structured_fields>",
            &format!("<structured_fields><session_intent>bugfix</session_intent><session_topic>{topic}</session_topic>"));
        let result = process_with_summarizer(&mut conn, &task, |_| async { Ok(response) }).await?;
        assert_eq!(result, SessionRollupResult::Written);
        let label = conn.query_row(
            "SELECT session_intent, session_topic, session_intent_source FROM session_summaries",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )?;
        assert_eq!(label, (None, None, None), "topic={topic}");
        let promoted: String = conn.query_row(
            "SELECT review_status FROM memory_candidates WHERE text = ?1",
            [&candidate],
            |row| row.get(0),
        )?;
        assert_eq!(promoted, "auto_promoted");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE content = ?1",
            [&candidate],
            |row| row.get(0),
        )?;
        assert_eq!(count, 1);
    }
    Ok(())
}
