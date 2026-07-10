use std::path::Path;

use anyhow::Result;
use rusqlite::params;

use super::*;

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set_path(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.as_ref() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn custom_capture(
    conn: &Connection,
    session_id: &str,
    project: &str,
    cwd: Option<&str>,
    content: &str,
) -> Result<i64> {
    let outcome = record_captured_event(
        conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id,
            project,
            cwd,
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content,
            task_kind: Some(ExtractionTaskKind::SessionRollup),
        },
    )?;
    outcome
        .extraction_task_id
        .ok_or_else(|| anyhow::anyhow!("expected extraction task id"))
}

fn job_types(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT job_type FROM jobs ORDER BY id ASC")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn job_payloads(conn: &Connection, job_type: &str) -> Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT payload_json FROM jobs WHERE job_type = ?1 ORDER BY id ASC")?;
    let rows = stmt.query_map([job_type], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn insert_injected_test_memory(
    conn: &Connection,
    project: &str,
    session_id: &str,
    suffix: &str,
) -> Result<i64> {
    let title = format!("{suffix} rollout decision");
    let memory_id = crate::memory::insert_memory(
        conn,
        Some("seed-session"),
        project,
        None,
        &title,
        "Keep the capture-ledger rollup path fail closed.",
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

fn failure_citation_transcript(memory_id: i64) -> String {
    [
        transcript_message(
            "assistant",
            "cargo check failed with the same compiler error after the third attempted fix",
        ),
        transcript_message(
            "user",
            "Lesson: after three consecutive failed fixes, stop and challenge the hypothesis before editing again",
        ),
        transcript_message(
            "assistant",
            format!("Used the rollout decision.\nMemory citations: memory:#{memory_id}"),
        ),
    ]
    .join("\n")
}

#[tokio::test]
async fn session_rollup_enqueues_followup_jobs_after_rollup() -> Result<()> {
    let mut conn = setup_conn();
    custom_capture(
        &conn,
        "sess-rollup-followups",
        "/tmp/remem",
        Some("/tmp/remem"),
        r#"{"session_id":"sess-rollup-followups","cwd":"/tmp/remem","remem_ai_profile":"custom"}"#,
    )?;
    let task = claim_rollup_task(&mut conn)?;

    let result = process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response_with_structured_fields(
            "Rollup completed follow-up scheduling.",
            "Retire Summary followups",
            "SessionRollup queues Compress and Dream after summary persistence.",
            "",
            "Keep Compress and Dream behind rollup completion.",
            "",
            "",
        ))
    })
    .await?;

    assert_eq!(result, SessionRollupResult::Written);
    assert_eq!(
        job_types(&conn)?,
        vec!["compress".to_string(), "dream".to_string()]
    );
    let summary_jobs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE job_type = 'summary'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(summary_jobs, 0);
    let dream_payload: serde_json::Value = serde_json::from_str(&job_payloads(&conn, "dream")?[0])?;
    assert_eq!(dream_payload["remem_ai_profile"].as_str(), Some("custom"));
    Ok(())
}

#[tokio::test]
async fn session_rollup_rehomes_finalize_side_effects() -> Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("session-rollup-side-effects");
    std::fs::create_dir_all(&data_dir.path)?;
    let home = data_dir.path.join("home");
    std::fs::create_dir_all(&home)?;
    let _home = EnvVarGuard::set_path("HOME", &home);
    let _native_sync =
        EnvVarGuard::remove(crate::context::claude_memory::DISABLE_NATIVE_MEMORY_SYNC_ENV);
    let project_path = data_dir.path.join("project");
    let cwd_path = project_path.join("nested");
    std::fs::create_dir_all(&cwd_path)?;
    let git_status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&project_path)
        .status()?;
    assert!(git_status.success());
    let cwd = std::fs::canonicalize(&cwd_path)?
        .to_string_lossy()
        .to_string();
    let project = crate::db::project_from_cwd(&cwd);
    let memory_dir = home
        .join(".claude")
        .join("projects")
        .join(cwd.replace('/', "-"))
        .join("memory");
    std::fs::create_dir_all(&memory_dir)?;
    let mut conn = crate::db::open_db()?;
    custom_capture(
        &conn,
        "sess-rollup-finalize-effects",
        &project,
        Some(&cwd),
        &serde_json::json!({
            "session_id": "sess-rollup-finalize-effects",
            "cwd": cwd,
            "last_assistant_message": "rollup side effect source"
        })
        .to_string(),
    )?;
    let task = claim_rollup_task(&mut conn)?;

    process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response_with_structured_fields(
            "Rollup persisted final side effects.",
            "SessionRollup finalizes side effects",
            "SessionRollup now creates decision candidates after persistence.",
            "The rollup worker owns native memory sync.",
            "Keep candidate, workstream, and native sync side effects on rollup.",
            "Prefer worker-side side effects after Summary retirement.",
            "",
        ))
    })
    .await?;

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    assert!(candidate_count > 0);
    let workstream_title: String = conn.query_row(
        "SELECT title FROM workstreams WHERE project = ?1",
        params![project],
        |row| row.get(0),
    )?;
    assert_eq!(workstream_title, "SessionRollup finalizes side effects");
    let native_file = memory_dir.join(crate::context::claude_memory::REMEM_FILE);
    let content = std::fs::read_to_string(&native_file)?;
    assert!(
        content.contains("SessionRollup finalizes side effects"),
        "{content}"
    );
    assert!(
        content.contains("SessionRollup now creates decision candidates after persistence."),
        "{content}"
    );
    Ok(())
}

#[tokio::test]
async fn session_rollup_worker_drains_raw_archive_from_stop_payload() -> Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("session-rollup-raw-archive");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"archived assistant turn"}]}}"#,
    )?;
    let mut conn = crate::db::open_db()?;
    custom_capture(
        &conn,
        "sess-rollup-raw",
        "/tmp/remem",
        Some("/tmp/remem"),
        &serde_json::json!({
            "session_id": "sess-rollup-raw",
            "cwd": "/tmp/remem",
            "transcript_path": transcript
        })
        .to_string(),
    )?;
    let task = claim_rollup_task(&mut conn)?;

    process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response("Raw archive moved to worker.", ""))
    })
    .await?;

    let (source, content): (String, String) = conn.query_row(
        "SELECT source, content FROM raw_messages WHERE session_id = ?1",
        ["sess-rollup-raw"],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(source, crate::memory::raw_archive::SOURCE_TRANSCRIPT);
    assert_eq!(content, "archived assistant turn");
    Ok(())
}

#[tokio::test]
async fn session_rollup_preserves_transcript_backed_stop_memory_side_effects() -> Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("session-rollup-transcript-side-effects");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("transcript.jsonl");
    let mut conn = crate::db::open_db()?;
    let project = "/tmp/remem";
    let session_id = "sess-rollup-transcript-side-effects";
    let memory_id = insert_injected_test_memory(&conn, project, session_id, "transcript")?;
    let transcript_text = failure_citation_transcript(memory_id);
    std::fs::write(&transcript, transcript_text)?;
    let transcript_byte_len = std::fs::metadata(&transcript)?.len();
    custom_capture(
        &conn,
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
    let task = claim_rollup_task(&mut conn)?;

    let result = process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response(
            "Transcript-backed side effects are preserved.",
            "",
        ))
    })
    .await?;

    assert_eq!(result, SessionRollupResult::Written);
    let usage_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_usage_events WHERE memory_id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    let failure_lessons: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_lesson_feed_events
         WHERE project = ?1 AND session_id = ?2",
        params![project, session_id],
        |row| row.get(0),
    )?;
    assert_eq!(usage_events, 1);
    assert_eq!(failure_lessons, 1);
    Ok(())
}

#[tokio::test]
async fn session_rollup_honors_stop_transcript_snapshot_boundary() -> Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("session-rollup-transcript-boundary");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("transcript.jsonl");
    let mut conn = crate::db::open_db()?;
    let project = "/tmp/remem";
    let session_id = "sess-rollup-transcript-boundary";
    let before_memory = insert_injected_test_memory(&conn, project, session_id, "before-stop")?;
    let after_memory = insert_injected_test_memory(&conn, project, session_id, "after-stop")?;
    let before = transcript_message(
        "assistant",
        format!("Before Stop.\nMemory citations: memory:#{before_memory}"),
    );
    let after = transcript_message(
        "assistant",
        format!("After Stop.\nMemory citations: memory:#{after_memory}"),
    );
    std::fs::write(&transcript, format!("{before}\n"))?;
    let transcript_byte_len = std::fs::metadata(&transcript)?.len();
    custom_capture(
        &conn,
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
    std::fs::write(&transcript, format!("{before}\n{after}\n"))?;
    let task = claim_rollup_task(&mut conn)?;

    let result = process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response("Only the Stop snapshot is eligible.", ""))
    })
    .await?;

    assert_eq!(result, SessionRollupResult::Written);
    let before_usage: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_usage_events WHERE memory_id = ?1",
        [before_memory],
        |row| row.get(0),
    )?;
    let after_usage: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_usage_events WHERE memory_id = ?1",
        [after_memory],
        |row| row.get(0),
    )?;
    let raw_after_stop: i64 = conn.query_row(
        "SELECT COUNT(*) FROM raw_messages WHERE content LIKE 'After Stop.%'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(before_usage, 1);
    assert_eq!(after_usage, 0);
    assert_eq!(raw_after_stop, 0);
    Ok(())
}

#[tokio::test]
async fn session_rollup_retries_transcript_side_effects_without_resummarizing() -> Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("session-rollup-transcript-retry");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("transcript.jsonl");
    let mut conn = crate::db::open_db()?;
    let project = "/tmp/remem";
    let session_id = "sess-rollup-transcript-retry";
    let memory_id = insert_injected_test_memory(&conn, project, session_id, "retry")?;
    std::fs::write(&transcript, failure_citation_transcript(memory_id))?;
    let transcript_byte_len = std::fs::metadata(&transcript)?.len();
    custom_capture(
        &conn,
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
    let task = claim_rollup_task(&mut conn)?;
    conn.execute_batch(
        "CREATE TRIGGER fail_rollup_failure_lesson
         BEFORE INSERT ON memory_lesson_feed_events
         BEGIN
             SELECT RAISE(FAIL, 'forced failure lesson error');
         END;",
    )?;

    let first = process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response(
            "Persist once before retrying Stop side effects.",
            "",
        ))
    })
    .await;
    let first_error = match first {
        Ok(result) => anyhow::bail!("failure-lesson fault unexpectedly returned {result:?}"),
        Err(error) => error,
    };
    assert!(first_error.to_string().contains("failure-lesson"));
    assert_eq!(summary_count(&conn), 1);
    assert_eq!(
        job_types(&conn)?,
        vec!["compress".to_string(), "dream".to_string()]
    );

    conn.execute_batch(
        "DROP TRIGGER fail_rollup_failure_lesson;
         CREATE TRIGGER fail_rollup_memory_citation
         BEFORE INSERT ON memory_citation_events
         BEGIN
             SELECT RAISE(FAIL, 'forced memory citation error');
         END;",
    )?;
    let second = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("persisted rollup retry must not call the summarizer")
    })
    .await;
    let second_error = match second {
        Ok(result) => anyhow::bail!("memory-citation fault unexpectedly returned {result:?}"),
        Err(error) => error,
    };
    assert!(second_error.to_string().contains("memory-citation"));
    assert_eq!(summary_count(&conn), 1);

    conn.execute_batch("DROP TRIGGER fail_rollup_memory_citation;")?;
    let retry_result = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("persisted rollup retry must not call the summarizer")
    })
    .await?;
    assert_eq!(retry_result, SessionRollupResult::AlreadyExists);
    let lesson_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_lesson_feed_events
         WHERE project = ?1 AND session_id = ?2",
        params![project, session_id],
        |row| row.get(0),
    )?;
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
    assert_eq!(lesson_events, 1);
    assert_eq!(citation_events, 1);
    assert_eq!(usage_events, 1);
    assert_eq!(
        job_types(&conn)?,
        vec!["compress".to_string(), "dream".to_string()]
    );
    Ok(())
}

#[tokio::test]
async fn session_rollup_later_range_uses_its_own_persisted_fields() -> Result<()> {
    let mut conn = setup_conn();
    custom_capture(
        &conn,
        "sess-rollup-range-fields",
        "/tmp/remem",
        Some("/tmp/remem"),
        r#"{"session_id":"sess-rollup-range-fields","cwd":"/tmp/remem","last_assistant_message":"first"}"#,
    )?;
    let first_task = claim_rollup_task(&mut conn)?;
    let first_result = process_with_summarizer(&mut conn, &first_task, |_prompt| async {
        Ok(xml_response_with_structured_fields(
            "First range summary.",
            "First range request",
            "First range decision is intentionally long enough to promote.",
            "",
            "First range next step",
            "",
            "",
        ))
    })
    .await?;
    assert_eq!(first_result, SessionRollupResult::Written);
    db::mark_extraction_task_done(
        &conn,
        first_task.id,
        "worker-a",
        first_task.high_watermark_event_id,
    )?;

    custom_capture(
        &conn,
        "sess-rollup-range-fields",
        "/tmp/remem",
        Some("/tmp/remem"),
        r#"{"session_id":"sess-rollup-range-fields","cwd":"/tmp/remem","last_assistant_message":"second"}"#,
    )?;
    let second_task = claim_rollup_task(&mut conn)?;
    let second_result = process_with_summarizer(&mut conn, &second_task, |_prompt| async {
        Ok(xml_response_with_structured_fields(
            "Second range summary.",
            "Second range request",
            "Second range decision must drive the second side-effect pass.",
            "",
            "Second range next step",
            "",
            "",
        ))
    })
    .await?;
    assert_eq!(second_result, SessionRollupResult::Written);

    let second_workstream_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM workstreams
         WHERE project = '/tmp/remem' AND title = 'Second range request'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(second_workstream_count, 1);
    let second_candidate_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_candidates
         WHERE text LIKE '%Second range decision%'",
        [],
        |row| row.get(0),
    )?;
    assert!(second_candidate_count > 0);
    Ok(())
}

#[tokio::test]
async fn session_rollup_retries_persisted_side_effect_failures() -> Result<()> {
    let mut conn = setup_conn();
    custom_capture(
        &conn,
        "sess-rollup-side-effect-retry",
        "/tmp/remem",
        Some("/tmp/remem"),
        r#"{"session_id":"sess-rollup-side-effect-retry","cwd":"/tmp/remem"}"#,
    )?;
    let task = claim_rollup_task(&mut conn)?;
    conn.execute_batch(
        "CREATE TRIGGER fail_rollup_workstream
         BEFORE INSERT ON workstreams
         BEGIN
             SELECT RAISE(FAIL, 'forced workstream failure');
         END;",
    )?;

    let first_error = process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response_with_structured_fields(
            "Persist the summary before retrying side effects.",
            "Retry rollup side effects",
            "Side effects must not be silently omitted after summary persistence.",
            "",
            "Retry the same persisted rollup range.",
            "",
            "",
        ))
    })
    .await
    .expect_err("workstream persistence failure must keep the task retryable");
    assert!(first_error.to_string().contains("workstream"));
    assert_eq!(summary_count(&conn), 1);

    conn.execute_batch("DROP TRIGGER fail_rollup_workstream;")?;
    let retry_result = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("existing rollup retry must not call the summarizer")
    })
    .await?;
    assert_eq!(retry_result, SessionRollupResult::AlreadyExists);
    let workstream_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM workstreams WHERE title = 'Retry rollup side effects'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(workstream_count, 1);
    Ok(())
}

#[tokio::test]
async fn session_rollup_retries_incomplete_raw_archive_ingest() -> Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("session-rollup-raw-retry");
    std::fs::create_dir_all(&data_dir.path)?;
    let transcript = data_dir.path.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"retry archived assistant turn"}]}}"#,
    )?;
    let mut conn = crate::db::open_db()?;
    custom_capture(
        &conn,
        "sess-rollup-raw-retry",
        "/tmp/remem",
        Some("/tmp/remem"),
        &serde_json::json!({
            "session_id": "sess-rollup-raw-retry",
            "cwd": "/tmp/remem",
            "transcript_path": transcript
        })
        .to_string(),
    )?;
    let task = claim_rollup_task(&mut conn)?;
    conn.execute_batch(
        "CREATE TRIGGER fail_rollup_raw_insert
         BEFORE INSERT ON raw_messages
         BEGIN
             SELECT RAISE(FAIL, 'forced raw archive failure');
         END;",
    )?;

    let first_error = process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response(
            "Persist summary while raw archive retries.",
            "",
        ))
    })
    .await
    .expect_err("partial raw archive ingest must keep the task retryable");
    assert!(first_error
        .to_string()
        .contains("raw archive ingest incomplete"));
    assert_eq!(summary_count(&conn), 1);
    let premature_jobs: i64 = conn.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))?;
    assert_eq!(premature_jobs, 0);
    let failure_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM raw_ingest_failures
         WHERE session_id = 'sess-rollup-raw-retry'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(failure_count, 1);

    conn.execute_batch("DROP TRIGGER fail_rollup_raw_insert;")?;
    let retry_result = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("existing rollup retry must not call the summarizer")
    })
    .await?;
    assert_eq!(retry_result, SessionRollupResult::AlreadyExists);
    assert_eq!(
        job_types(&conn)?,
        vec!["compress".to_string(), "dream".to_string()]
    );
    let raw_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM raw_messages
         WHERE session_id = 'sess-rollup-raw-retry'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(raw_count, 1);
    Ok(())
}
