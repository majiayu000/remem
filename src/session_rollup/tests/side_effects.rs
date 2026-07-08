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
