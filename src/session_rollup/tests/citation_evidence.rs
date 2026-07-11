use anyhow::Result;
use rusqlite::params;

use super::*;

fn insert_injected_memory(
    conn: &Connection,
    project: &str,
    session_id: &str,
    suffix: &str,
) -> Result<i64> {
    let title = format!("{suffix} citation evidence");
    let memory_id = crate::memory::insert_memory(
        conn,
        Some("seed-session"),
        project,
        None,
        &title,
        "Preserve exact Stop citation facts independently of prompt budgets.",
        "decision",
        None,
    )?;
    conn.execute(
        "INSERT INTO context_injection_items
         (injection_run_id, host, project, session_id, injection_key, output_mode,
          decision, item_kind, item_id, memory_id, channel, render_order, status,
          title, provenance, staleness, injected_at_epoch)
         VALUES (?1, 'codex-cli', ?2, ?3, ?4, 'full',
                 'emitted', 'memory', ?5, ?5, 'core', 1, 'injected',
                 ?6, 'src=memory', 'current', 100)",
        params![
            format!("run-{suffix}"),
            project,
            session_id,
            format!("key-{suffix}"),
            memory_id,
            title
        ],
    )?;
    Ok(memory_id)
}

fn transcript_message(role: &str, text: impl Into<String>) -> String {
    serde_json::json!({
        "type": role,
        "message": {"content": [{"type": "text", "text": text.into()}]}
    })
    .to_string()
}

fn capture_transcript_stop(
    conn: &Connection,
    session_id: &str,
    transcript: &std::path::Path,
) -> Result<()> {
    let transcript_byte_len = std::fs::metadata(transcript)?.len();
    capture(
        conn,
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
    Ok(())
}

async fn persist_then_retry_without_sources(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    transcripts: &[&std::path::Path],
) -> Result<SessionRollupResult> {
    conn.execute_batch(
        "CREATE TRIGGER fail_rollup_citation_snapshot_lesson
         BEFORE INSERT ON memory_lesson_feed_events
         BEGIN
             SELECT RAISE(FAIL, 'forced citation snapshot lesson error');
         END;",
    )?;
    let first = process_with_summarizer(conn, task, |_prompt| async {
        Ok(xml_response(
            "Persist citation evidence before retrying.",
            "",
        ))
    })
    .await;
    let first_error = match first {
        Ok(result) => anyhow::bail!("forced side-effect failure returned {result:?}"),
        Err(error) => error,
    };
    assert!(first_error.to_string().contains("failure-lesson"));
    assert_eq!(summary_count(conn), 1);

    for transcript in transcripts {
        std::fs::remove_file(transcript)?;
    }
    conn.execute_batch("DROP TRIGGER fail_rollup_citation_snapshot_lesson;")?;

    process_with_summarizer(conn, task, |_prompt| async {
        anyhow::bail!("persisted rollup retry must not call the summarizer")
    })
    .await
}

#[tokio::test]
async fn persisted_citation_evidence_keeps_long_assistant_tail() -> Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("session-rollup-long-citation-evidence");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("long-tail.jsonl");
    let mut conn = crate::db::open_db()?;
    let project = "/tmp/remem";
    let session_id = "sess-rollup-long-citation-evidence";
    let memory_id = insert_injected_memory(&conn, project, session_id, "long-tail")?;
    let long_assistant = format!(
        "{}\nMemory citations: memory:#{memory_id}",
        "This assistant evidence precedes its citation contract. ".repeat(200)
    );
    assert!(long_assistant.len() > 8 * 1024);
    std::fs::write(
        &transcript,
        [
            transcript_message(
                "assistant",
                "cargo check failed with the same compiler error after the third attempted fix",
            ),
            transcript_message(
                "user",
                "Lesson: stop and challenge the hypothesis after three consecutive failed fixes",
            ),
            transcript_message("assistant", long_assistant),
        ]
        .join("\n"),
    )?;
    capture_transcript_stop(&conn, session_id, &transcript)?;
    let task = claim_rollup_task(&mut conn)?;

    let result = persist_then_retry_without_sources(&mut conn, &task, &[&transcript]).await?;

    assert_eq!(result, SessionRollupResult::AlreadyExists);
    let usage_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_usage_events WHERE memory_id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(usage_events, 1);
    Ok(())
}

#[tokio::test]
async fn persisted_citation_evidence_survives_cross_stop_prompt_eviction() -> Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new(
        "session-rollup-cross-stop-citation-evidence",
    );
    std::fs::create_dir_all(&data_dir.path)?;
    let earlier_transcript = data_dir.path.join("earlier.jsonl");
    let later_transcript = data_dir.path.join("later.jsonl");
    let mut conn = crate::db::open_db()?;
    let project = "/tmp/remem";
    let session_id = "sess-rollup-cross-stop-citation-evidence";
    let memory_id = insert_injected_memory(&conn, project, session_id, "earlier-stop")?;
    let earlier_citation = format!(
        "{}\nMemory citations: memory:#{memory_id}",
        "Earlier Stop assistant evidence. ".repeat(240)
    );
    assert!(earlier_citation.len() < 8 * 1024);
    std::fs::write(
        &earlier_transcript,
        [
            transcript_message(
                "assistant",
                format!(
                    "cargo check failed three times before the hypothesis changed. {}",
                    "failure context ".repeat(700)
                ),
            ),
            transcript_message(
                "user",
                "Lesson: challenge the hypothesis after three consecutive failed fixes",
            ),
            transcript_message("assistant", earlier_citation),
        ]
        .join("\n"),
    )?;
    let later_messages = (0..8)
        .map(|index| {
            transcript_message(
                "assistant",
                format!(
                    "Later Stop evidence {index}: {}",
                    "later context ".repeat(700)
                ),
            )
        })
        .collect::<Vec<_>>();
    std::fs::write(&later_transcript, later_messages.join("\n"))?;
    capture_transcript_stop(&conn, session_id, &earlier_transcript)?;
    capture_transcript_stop(&conn, session_id, &later_transcript)?;
    let task = claim_rollup_task(&mut conn)?;

    let range = load_rollup_range(&conn, &task)?
        .ok_or_else(|| anyhow::anyhow!("rollup range should load"))?;
    let evidence = super::super::transcript_evidence::load_prompt_transcript_evidence(&range)?;
    assert!(evidence.truncated);
    assert!(!evidence.messages.iter().any(|message| {
        message.source_event_id == range.events[0].id
            && message.content.contains("Memory citations:")
    }));

    let result = persist_then_retry_without_sources(
        &mut conn,
        &task,
        &[&earlier_transcript, &later_transcript],
    )
    .await?;

    assert_eq!(result, SessionRollupResult::AlreadyExists);
    let usage_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_usage_events WHERE memory_id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(usage_events, 1);
    Ok(())
}

#[tokio::test]
async fn persisted_citation_evidence_covers_each_boundary_of_repeated_path() -> Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new(
        "session-rollup-repeated-path-citation-evidence",
    );
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("repeated.jsonl");
    let mut conn = crate::db::open_db()?;
    let project = "/tmp/remem";
    let session_id = "sess-rollup-repeated-path-citation-evidence";
    let earlier_memory = insert_injected_memory(&conn, project, session_id, "earlier-boundary")?;
    let later_memory = insert_injected_memory(&conn, project, session_id, "later-boundary")?;
    let earlier_content = [
        transcript_message(
            "assistant",
            "cargo check failed with the same compiler error after the third attempted fix",
        ),
        transcript_message(
            "user",
            "Lesson: challenge the hypothesis after three consecutive failed fixes",
        ),
        transcript_message(
            "assistant",
            format!("Earlier boundary.\nMemory citations: memory:#{earlier_memory}"),
        ),
    ]
    .join("\n");
    std::fs::write(&transcript, &earlier_content)?;
    capture_transcript_stop(&conn, session_id, &transcript)?;

    let later_content = format!(
        "{earlier_content}\n{}",
        transcript_message(
            "assistant",
            format!("Later boundary.\nMemory citations: memory:#{later_memory}"),
        )
    );
    std::fs::write(&transcript, later_content)?;
    capture_transcript_stop(&conn, session_id, &transcript)?;
    let task = claim_rollup_task(&mut conn)?;

    let result = persist_then_retry_without_sources(&mut conn, &task, &[&transcript]).await?;

    assert_eq!(result, SessionRollupResult::AlreadyExists);
    for memory_id in [earlier_memory, later_memory] {
        let usage_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_usage_events WHERE memory_id = ?1",
            [memory_id],
            |row| row.get(0),
        )?;
        assert_eq!(usage_events, 1, "memory {memory_id} should be cited");
    }
    Ok(())
}

#[tokio::test]
async fn legacy_v066_citation_message_hash_stays_idempotent() -> Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("session-rollup-v066-citation-hash");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("legacy-v066.jsonl");
    let mut conn = crate::db::open_db()?;
    let project = "/tmp/remem";
    let session_id = "sess-rollup-v066-citation-hash";
    let memory_id = insert_injected_memory(&conn, project, session_id, "legacy-v066")?;
    let full_assistant =
        format!("password=hunter2\nUsed the decision.\nMemory citations: memory:#{memory_id}");
    std::fs::write(
        &transcript,
        [
            transcript_message(
                "assistant",
                "cargo check failed with the same compiler error after the third attempted fix",
            ),
            transcript_message(
                "user",
                "Lesson: challenge the hypothesis after three consecutive failed fixes",
            ),
            transcript_message("assistant", &full_assistant),
        ]
        .join("\n"),
    )?;
    capture_transcript_stop(&conn, session_id, &transcript)?;
    let task = claim_rollup_task(&mut conn)?;
    let range = load_rollup_range(&conn, &task)?
        .ok_or_else(|| anyhow::anyhow!("rollup range should load"))?;
    let stop_event_id = range
        .events
        .iter()
        .find(|event| event.event_type == "session_stop")
        .map(|event| event.id)
        .ok_or_else(|| anyhow::anyhow!("Stop event should load"))?;
    conn.execute_batch(
        "CREATE TRIGGER fail_rollup_v066_citation_lesson
         BEFORE INSERT ON memory_lesson_feed_events
         BEGIN
             SELECT RAISE(FAIL, 'forced v066 citation lesson error');
         END;",
    )?;
    let first = process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response("Persist the pre-citation rollup state.", ""))
    })
    .await;
    let first_error = match first {
        Ok(result) => anyhow::bail!("forced side-effect failure returned {result:?}"),
        Err(error) => error,
    };
    assert!(first_error.to_string().contains("failure-lesson"));
    conn.execute_batch("DROP TRIGGER fail_rollup_v066_citation_lesson;")?;

    let legacy_assistant = crate::adapter::common::redact_sensitive_text(&full_assistant);
    assert_ne!(legacy_assistant, full_assistant);
    let legacy_evidence = serde_json::json!({
        "messages": [{
            "source_event_id": stop_event_id,
            "role": "assistant",
            "content": legacy_assistant
        }],
        "truncated": false
    })
    .to_string();
    conn.execute(
        "UPDATE session_summaries SET transcript_evidence_json = ?1
         WHERE session_row_id = ?2
           AND covered_from_event_id = ?3
           AND covered_to_event_id = ?4",
        params![
            legacy_evidence,
            task.session_row_id,
            range.from_event_id,
            range.to_event_id
        ],
    )?;
    crate::summarize::record_stop_memory_citation_usage(
        &conn,
        &task.host,
        project,
        session_id,
        &legacy_assistant,
    )?;

    let result = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("persisted rollup retry must not call the summarizer")
    })
    .await?;

    assert_eq!(result, SessionRollupResult::AlreadyExists);
    let citation_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_citation_events
         WHERE project = ?1 AND session_id = ?2",
        params![project, session_id],
        |row| row.get(0),
    )?;
    let usage_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_usage_events WHERE memory_id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    let access_count: i64 = conn.query_row(
        "SELECT access_count FROM memories WHERE id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(citation_events, 1);
    assert_eq!(usage_events, 1);
    assert_eq!(access_count, 1);
    Ok(())
}
