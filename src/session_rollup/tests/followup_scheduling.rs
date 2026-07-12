use anyhow::Result;
use rusqlite::{params, Connection};

use super::side_effects::{
    custom_capture, failure_citation_transcript, insert_injected_test_memory, job_types,
};
use super::*;

async fn persist_rollup_with_retryable_stop_failure(
    conn: &mut Connection,
    data_dir: &crate::db::test_support::ScopedTestDataDir,
    session_id: &str,
) -> Result<db::ExtractionTask> {
    let project = "/tmp/remem";
    let transcript = data_dir.path.join(format!("{session_id}.jsonl"));
    let memory_id = insert_injected_test_memory(conn, project, session_id, session_id)?;
    std::fs::write(&transcript, failure_citation_transcript(memory_id))?;
    let transcript_byte_len = std::fs::metadata(&transcript)?.len();
    custom_capture(
        conn,
        session_id,
        project,
        Some(project),
        &serde_json::json!({
            "session_id": session_id,
            "cwd": project,
            "transcript_path": transcript,
            "transcript_byte_len": transcript_byte_len
        })
        .to_string(),
    )?;
    let task = claim_rollup_task(conn)?;
    conn.execute_batch(
        "CREATE TRIGGER fail_followup_test_lesson
         BEFORE INSERT ON memory_lesson_feed_events
         BEGIN
             SELECT RAISE(FAIL, 'forced post-persistence failure');
         END;",
    )?;

    let error = process_with_summarizer(conn, &task, |_prompt| async {
        Ok(xml_response(
            "Persist the rollup and schedule maintenance once.",
            "",
        ))
    })
    .await
    .expect_err("post-persistence Stop failure must keep the rollup retryable");
    assert!(error.to_string().contains("failure-lesson"));
    assert_eq!(summary_count(conn), 1);
    assert_eq!(job_types(conn)?, ["compress", "dream"]);
    conn.execute_batch("DROP TRIGGER fail_followup_test_lesson;")?;
    Ok(task)
}

fn job_count(conn: &Connection, job_type: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE job_type = ?1",
        [job_type],
        |row| row.get(0),
    )?)
}

fn followup_checkpoint(conn: &Connection, task: &db::ExtractionTask) -> Result<Option<i64>> {
    let session_row_id = task
        .session_row_id
        .ok_or_else(|| anyhow::anyhow!("rollup task missing session row id"))?;
    let to_event_id = task
        .high_watermark_event_id
        .ok_or_else(|| anyhow::anyhow!("rollup task missing high watermark"))?;
    Ok(conn.query_row(
        "SELECT followup_scheduling_completed_at_epoch
         FROM session_summaries
         WHERE session_row_id = ?1
           AND covered_from_event_id = ?2
           AND covered_to_event_id = ?3",
        params![
            session_row_id,
            task.cursor_event_id.unwrap_or(0).saturating_add(1),
            to_event_id
        ],
        |row| row.get(0),
    )?)
}

#[tokio::test]
async fn session_rollup_followup_scheduling_survives_completed_compress_before_retry() -> Result<()>
{
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("rollup-followup-compress-retry");
    let mut conn = crate::db::open_db()?;
    let task = persist_rollup_with_retryable_stop_failure(
        &mut conn,
        &data_dir,
        "sess-followup-compress-retry",
    )
    .await?;
    conn.execute(
        "UPDATE jobs
         SET state = 'done', updated_at_epoch = updated_at_epoch + 1
         WHERE job_type = 'compress'",
        [],
    )?;

    let retry = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("persisted retry must not call the summarizer")
    })
    .await?;

    assert_eq!(retry, SessionRollupResult::AlreadyExists);
    assert_eq!(job_count(&conn, "compress")?, 1);
    assert_eq!(job_count(&conn, "dream")?, 1);
    assert!(followup_checkpoint(&conn, &task)?.is_some());
    Ok(())
}

#[tokio::test]
async fn session_rollup_followup_scheduling_preserves_failed_dream_for_same_range() -> Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("rollup-followup-dream-retry");
    let mut conn = crate::db::open_db()?;
    let task = persist_rollup_with_retryable_stop_failure(
        &mut conn,
        &data_dir,
        "sess-followup-dream-retry",
    )
    .await?;
    conn.execute(
        "UPDATE jobs
         SET state = 'failed', attempt_count = max_attempts,
             last_error = 'forced dream terminal failure',
             failure_class = 'permanent',
             updated_at_epoch = updated_at_epoch + 1
         WHERE job_type = 'dream'",
        [],
    )?;

    let retry = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("persisted retry must not call the summarizer")
    })
    .await?;

    assert_eq!(retry, SessionRollupResult::AlreadyExists);
    assert_eq!(job_count(&conn, "compress")?, 1);
    assert_eq!(job_count(&conn, "dream")?, 1);
    assert!(followup_checkpoint(&conn, &task)?.is_some());
    let diagnostics: (String, i64, String, String) = conn.query_row(
        "SELECT state, attempt_count, last_error, failure_class
         FROM jobs WHERE job_type = 'dream'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        diagnostics,
        (
            "failed".to_string(),
            6,
            "forced dream terminal failure".to_string(),
            "permanent".to_string()
        )
    );
    Ok(())
}

#[tokio::test]
async fn session_rollup_new_range_gets_new_followup_scheduling_decision() -> Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("rollup-followup-new-range");
    let mut conn = crate::db::open_db()?;
    let session_id = "sess-followup-new-range";
    let first_task =
        persist_rollup_with_retryable_stop_failure(&mut conn, &data_dir, session_id).await?;
    conn.execute_batch(
        "UPDATE jobs
         SET state = CASE job_type WHEN 'compress' THEN 'done' ELSE 'failed' END,
             updated_at_epoch = updated_at_epoch + 1;",
    )?;
    db::mark_extraction_task_done(
        &conn,
        first_task.id,
        "worker-a",
        first_task.high_watermark_event_id,
    )?;

    custom_capture(
        &conn,
        session_id,
        "/tmp/remem",
        Some("/tmp/remem"),
        r#"{"session_id":"sess-followup-new-range","cwd":"/tmp/remem","last_assistant_message":"new range"}"#,
    )?;
    let second_task = claim_rollup_task(&mut conn)?;
    let second = process_with_summarizer(&mut conn, &second_task, |_prompt| async {
        Ok(xml_response("Schedule maintenance for the new range.", ""))
    })
    .await?;

    assert_eq!(second, SessionRollupResult::Written);
    assert_eq!(job_count(&conn, "compress")?, 2);
    assert_eq!(job_count(&conn, "dream")?, 2);
    let completed_decisions: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_summaries
         WHERE followup_scheduling_completed_at_epoch IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(completed_decisions, 2);
    assert!(followup_checkpoint(&conn, &second_task)?.is_some());
    Ok(())
}

#[tokio::test]
async fn session_rollup_followup_scheduling_rolls_back_partial_enqueue() -> Result<()> {
    let mut conn = setup_conn();
    custom_capture(
        &conn,
        "sess-followup-rollback",
        "/tmp/remem",
        Some("/tmp/remem"),
        r#"{"session_id":"sess-followup-rollback","cwd":"/tmp/remem"}"#,
    )?;
    let task = claim_rollup_task(&mut conn)?;
    conn.execute_batch(
        "CREATE TRIGGER fail_followup_dream_enqueue
         BEFORE INSERT ON jobs
         WHEN NEW.job_type = 'dream'
         BEGIN
             SELECT RAISE(FAIL, 'forced dream enqueue failure');
         END;",
    )?;

    let error = process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response(
            "Roll back a partial maintenance scheduling decision.",
            "",
        ))
    })
    .await
    .expect_err("Dream enqueue failure must roll back the scheduling transaction");
    assert!(error.to_string().contains("forced dream enqueue failure"));
    assert_eq!(summary_count(&conn), 1);
    assert_eq!(job_count(&conn, "compress")?, 0);
    assert_eq!(job_count(&conn, "dream")?, 0);
    assert_eq!(followup_checkpoint(&conn, &task)?, None);

    conn.execute_batch("DROP TRIGGER fail_followup_dream_enqueue;")?;
    let retry = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("persisted retry must not call the summarizer")
    })
    .await?;

    assert_eq!(retry, SessionRollupResult::AlreadyExists);
    assert_eq!(job_count(&conn, "compress")?, 1);
    assert_eq!(job_count(&conn, "dream")?, 1);
    assert!(followup_checkpoint(&conn, &task)?.is_some());
    Ok(())
}
