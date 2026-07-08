use std::time::Instant;

use anyhow::Result;

use crate::db;
use crate::db::project_from_cwd;
use crate::perf::{format_phase_timings, push_elapsed, time_result, time_value, PhaseTiming};

use super::super::constants::{
    SUMMARIZE_COOLDOWN_SECS, SUMMARIZE_LOCK_TIMEOUT_SECS, SUMMARY_PROMPT,
};
use super::super::input::{hash_message, SummarizeInput};
use super::super::parse::parse_summary;
use super::persist::{build_existing_summary_context, finalize_summary, sync_native_memory};
use super::side_effects::run_stop_hook_side_effects;

pub async fn process_summary_job_input(
    host: &str,
    profile: Option<&str>,
    input: &str,
) -> Result<()> {
    let total_start = Instant::now();
    let mut timings = Vec::new();
    let hook: SummarizeInput = time_result(&mut timings, "parse_payload", || {
        Ok(serde_json::from_str(input)?)
    })?;
    let Some(session_id) = hook.session_id.clone() else {
        return Ok(());
    };
    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = project_from_cwd(cwd);

    let mut conn = time_result(&mut timings, "db_open", db::open_db)?;

    let current_branch = time_value(&mut timings, "detect_branch", || db::detect_git_branch(cwd));
    let assistant_msg = time_result(&mut timings, "stop_hook_side_effects", || {
        run_stop_hook_side_effects(
            &conn,
            host,
            &hook,
            &session_id,
            &project,
            cwd,
            current_branch.as_deref(),
        )
    })?;

    let msg = time_value(&mut timings, "prepare_message", || {
        prepare_assistant_message(assistant_msg)
    });
    let Some(msg) = msg else {
        push_elapsed(&mut timings, "job_total", total_start);
        log_summary_job_timing("no_message", &project, &timings);
        return Ok(());
    };
    let msg_hash = hash_message(&msg);

    if time_result(&mut timings, "cooldown_check", || {
        db::is_summarize_on_cooldown(&conn, &project, SUMMARIZE_COOLDOWN_SECS)
    })? {
        crate::log::info(
            "summary-job",
            &format!("project={} on cooldown, skipping", project),
        );
        push_elapsed(&mut timings, "job_total", total_start);
        log_summary_job_timing("cooldown", &project, &timings);
        return Ok(());
    }

    if time_result(&mut timings, "duplicate_check", || {
        db::is_duplicate_message(&conn, &project, &msg_hash)
    })? {
        crate::log::info(
            "summary-job",
            &format!("project={} duplicate message, skipping", project),
        );
        push_elapsed(&mut timings, "job_total", total_start);
        log_summary_job_timing("duplicate", &project, &timings);
        return Ok(());
    }

    let memory_sid = time_result(&mut timings, "upsert_session", || {
        db::upsert_session(&conn, &session_id, &project, None)
    })?;
    let existing_ctx = time_result(&mut timings, "existing_context", || {
        build_existing_summary_context(&conn, &memory_sid, &project)
    })?;
    let user_message = format!(
        "{}Here is the assistant's last response from the session:\n\n{}",
        existing_ctx, msg
    );

    if !time_result(&mut timings, "lock_acquire", || {
        db::try_acquire_summarize_lock(&mut conn, &project, SUMMARIZE_LOCK_TIMEOUT_SECS)
    })? {
        crate::log::info(
            "summary-job",
            &format!("project={} summarize lock held, skipping", project),
        );
        push_elapsed(&mut timings, "job_total", total_start);
        log_summary_job_timing("lock_held", &project, &timings);
        return Ok(());
    }

    let payload_profile = profile_from_payload(input);
    let effective_profile = profile.or(payload_profile.as_deref());
    let ai_start = Instant::now();
    let response_result = call_summary_ai(
        host,
        effective_profile,
        &project,
        &session_id,
        &user_message,
    )
    .await;
    push_elapsed(&mut timings, "call_ai", ai_start);
    let response = match response_result {
        Ok(response) => response,
        Err(err) => {
            release_lock_or_log(&conn, &project, "ai-failure");
            push_elapsed(&mut timings, "job_total", total_start);
            log_summary_job_timing("ai_error", &project, &timings);
            return Err(anyhow::anyhow!("summary ai failed: {}", err));
        }
    };
    let Some(summary) = time_value(&mut timings, "parse_summary", || parse_summary(&response))
    else {
        release_lock_or_log(&conn, &project, "ai-skipped");
        crate::log::info("summary-job", "session skipped by AI");
        push_elapsed(&mut timings, "job_total", total_start);
        log_summary_job_timing("ai_skipped", &project, &timings);
        return Ok(());
    };

    time_result(&mut timings, "finalize_summary", || {
        finalize_summary(
            &mut conn,
            &session_id,
            &memory_sid,
            &project,
            &msg_hash,
            summary,
        )
    })?;
    time_value(&mut timings, "sync_native_memory", || {
        sync_native_memory(&conn, cwd, &project)
    });
    push_elapsed(&mut timings, "job_total", total_start);
    log_summary_job_timing("summarized", &project, &timings);
    Ok(())
}

fn release_lock_or_log(conn: &rusqlite::Connection, project: &str, reason: &str) {
    if let Err(e) = db::release_summarize_lock(conn, project) {
        crate::log::error(
            "summary-job",
            &format!(
                "[LOCK LEAK] failed to release summarize lock for {project} after {reason}: {e}"
            ),
        );
    }
}

fn prepare_assistant_message(message: String) -> Option<String> {
    if message.is_empty() || message.contains("<skip_summary") || message.len() < 50 {
        return None;
    }
    if message.len() > 12000 {
        Some(crate::db::truncate_str(&message, 12000).to_string())
    } else {
        Some(message)
    }
}

async fn call_summary_ai(
    host: &str,
    profile: Option<&str>,
    project: &str,
    session_id: &str,
    user_message: &str,
) -> Result<String> {
    let ai_start = std::time::Instant::now();
    let response = crate::ai::call_ai(
        SUMMARY_PROMPT,
        user_message,
        crate::ai::UsageContext {
            project: Some(project),
            session_id: Some(session_id),
            operation: "summarize",
            host: profile.is_none().then_some(host),
            profile,
        },
    )
    .await?;
    crate::log::info(
        "summary-job",
        &format!(
            "AI response {}ms {}B",
            ai_start.elapsed().as_millis(),
            response.len()
        ),
    );
    Ok(response)
}

fn profile_from_payload(input: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|value| {
            value
                .get("remem_ai_profile")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|profile| !profile.is_empty())
                .map(str::to_string)
        })
}

fn log_summary_job_timing(status: &str, project: &str, timings: &[PhaseTiming]) {
    crate::log::info(
        "summary-job-perf",
        &format!(
            "status={} project={} timings=[{}]",
            status,
            project,
            format_phase_timings(timings)
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::ScopedTestDataDir;
    use rusqlite::params;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
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

    #[tokio::test]
    async fn bad_transcript_path_uses_last_assistant_message_hook_fallback() -> Result<()> {
        let data_dir = ScopedTestDataDir::new("summary-raw-fallback");
        let missing_transcript = data_dir.path.join("missing-transcript.jsonl");
        let payload = serde_json::json!({
            "session_id": "session-raw-fallback",
            "cwd": data_dir.path.to_string_lossy(),
            "transcript_path": missing_transcript.to_string_lossy(),
            "last_assistant_message": "fallback assistant turn"
        });

        process_summary_job_input("codex-cli", None, &payload.to_string()).await?;

        let conn = db::open_db()?;
        let (role, source, content): (String, String, String) = conn.query_row(
            "SELECT role, source, content FROM raw_messages WHERE session_id = ?1",
            ["session-raw-fallback"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(role, crate::memory::raw_archive::ROLE_ASSISTANT);
        assert_eq!(source, crate::memory::raw_archive::SOURCE_HOOK);
        assert_eq!(content, "fallback assistant turn");

        let (path, kind): (String, String) = conn.query_row(
            "SELECT transcript_path, error_kind FROM raw_ingest_failures WHERE session_id = ?1",
            ["session-raw-fallback"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(path, missing_transcript.to_string_lossy());
        assert_eq!(kind, "read_error");
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_finalized_summary_syncs_native_memory_side_effect() -> Result<()> {
        let data_dir = ScopedTestDataDir::new("summary-native-memory-side-effect");
        std::fs::create_dir_all(&data_dir.path)?;
        let home = data_dir.path.join("home");
        std::fs::create_dir_all(&home)?;
        let _home = EnvVarGuard::set_path("HOME", &home);
        let _native_sync =
            EnvVarGuard::remove(crate::context::claude_memory::DISABLE_NATIVE_MEMORY_SYNC_ENV);

        let cwd_path = data_dir.path.join("project");
        std::fs::create_dir_all(&cwd_path)?;
        let cwd = std::fs::canonicalize(&cwd_path)?
            .to_string_lossy()
            .to_string();
        let project = db::project_from_cwd(&cwd);
        let memory_dir = home
            .join(".claude")
            .join("projects")
            .join(cwd.replace('/', "-"))
            .join("memory");
        std::fs::create_dir_all(&memory_dir)?;

        let stub_codex = data_dir.path.join("codex-summary-stub.sh");
        install_summary_stub(&stub_codex)?;
        crate::runtime_config::init_config()?;
        let stub_codex_path = stub_codex.to_string_lossy();
        crate::runtime_config::set_config_value("memory_ai.profiles.codex.path", &stub_codex_path)?;

        let conn = db::open_db()?;
        db::record_captured_event(
            &conn,
            &db::CaptureEventInput {
                host: "codex-cli",
                session_id: "session-native-memory-side-effect",
                project: &project,
                cwd: Some(&cwd),
                event_type: "session_stop",
                role: None,
                tool_name: None,
                content: "summary source payload for native memory side effect",
                task_kind: Some(db::ExtractionTaskKind::SessionRollup),
            },
        )?;
        drop(conn);

        let payload = serde_json::json!({
            "session_id": "session-native-memory-side-effect",
            "cwd": cwd,
            "last_assistant_message": "This assistant message is deliberately long enough for the legacy Summary job to call the summarization backend and finalize a row."
        });

        process_summary_job_input("codex-cli", None, &payload.to_string()).await?;

        let native_file = memory_dir.join(crate::context::claude_memory::REMEM_FILE);
        let content = std::fs::read_to_string(&native_file)?;
        assert!(content.contains("Summary native sync request"), "{content}");
        assert!(
            content.contains("Summary job kept native memory sync before retirement"),
            "{content}"
        );
        assert!(
            content.contains("Keep native memory sync owned before Summary retirement"),
            "{content}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn process_records_memory_citations_before_cooldown_skip() -> Result<()> {
        let (_data_dir, cwd, memory_id) =
            setup_cited_memory_session("summary-memory-citation", "session-citation")?;

        let message = format!(
            "This response is long enough for summary preparation and uses injected memory.\nMemory citations: memory:#{memory_id}"
        );
        let payload = serde_json::json!({
            "session_id": "session-citation",
            "cwd": cwd,
            "last_assistant_message": message
        });

        process_summary_job_input("codex-cli", None, &payload.to_string()).await?;

        assert_eq!(memory_usage_counts(memory_id)?, (1, 1, 1));
        Ok(())
    }

    #[tokio::test]
    async fn process_records_memory_citations_from_raw_tail_after_summary_truncation() -> Result<()>
    {
        let (_data_dir, cwd, memory_id) =
            setup_cited_memory_session("summary-memory-citation-tail", "session-citation-tail")?;
        let message = format!(
            "{}\nMemory citations: memory:#{memory_id}",
            "This long response pushes the citation past the summary truncation point. "
                .repeat(220)
        );
        assert!(message.len() > 12_000);
        let payload = serde_json::json!({
            "session_id": "session-citation-tail",
            "cwd": cwd,
            "last_assistant_message": message
        });

        process_summary_job_input("codex-cli", None, &payload.to_string()).await?;

        assert_eq!(memory_usage_counts(memory_id)?, (1, 1, 1));
        Ok(())
    }

    #[tokio::test]
    async fn process_records_memory_citations_before_summary_skip() -> Result<()> {
        let (_data_dir, cwd, memory_id) =
            setup_cited_memory_session("summary-memory-citation-skip", "session-citation-skip")?;
        let payload = serde_json::json!({
            "session_id": "session-citation-skip",
            "cwd": cwd,
            "last_assistant_message": format!("<skip_summary />\nMemory citations: memory:#{memory_id}")
        });

        process_summary_job_input("codex-cli", None, &payload.to_string()).await?;

        assert_eq!(memory_usage_counts(memory_id)?, (1, 1, 1));
        Ok(())
    }

    #[tokio::test]
    async fn process_distills_failure_lesson_before_cooldown_skip() -> Result<()> {
        let data_dir = ScopedTestDataDir::new("summary-failure-lesson-before-cooldown");
        std::fs::create_dir_all(&data_dir.path)?;
        let cwd = std::fs::canonicalize(&data_dir.path)?
            .to_string_lossy()
            .to_string();
        let project = db::project_from_cwd(&cwd);
        let transcript = data_dir.path.join("transcript.jsonl");
        std::fs::write(
            &transcript,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"cargo check failed with the same compiler error after the third attempted fix"}]}}
{"type":"user","message":{"content":[{"type":"text","text":"Lesson: after three consecutive failed fixes, stop and challenge the hypothesis before editing again"}]}}
"#,
        )?;
        let conn = db::open_db()?;
        conn.execute(
            "INSERT INTO summarize_cooldown(project, last_summarize_epoch, last_message_hash)
             VALUES (?1, ?2, NULL)",
            params![project, chrono::Utc::now().timestamp()],
        )?;
        drop(conn);
        let payload = serde_json::json!({
            "session_id": "session-failure-lesson-before-cooldown",
            "cwd": cwd,
            "transcript_path": transcript.to_string_lossy(),
            "last_assistant_message": "short"
        });

        process_summary_job_input("codex-cli", None, &payload.to_string()).await?;

        let conn = db::open_db()?;
        let (outcome_kind, failure_count): (String, i64) = conn.query_row(
            "SELECT l.outcome_kind, l.failure_count
             FROM memories m
             JOIN memory_lessons l ON l.memory_id = m.id
             WHERE m.memory_type = 'lesson'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(outcome_kind, "failure");
        assert_eq!(failure_count, 1);
        Ok(())
    }

    fn setup_cited_memory_session(
        test_name: &str,
        session_id: &str,
    ) -> Result<(ScopedTestDataDir, String, i64)> {
        let data_dir = ScopedTestDataDir::new(test_name);
        std::fs::create_dir_all(&data_dir.path)?;
        let cwd = std::fs::canonicalize(&data_dir.path)?
            .to_string_lossy()
            .to_string();
        let project = db::project_from_cwd(&cwd);
        let conn = db::open_db()?;
        let memory_id = crate::memory::insert_memory(
            &conn,
            Some("seed-session"),
            &project,
            None,
            "Usage target",
            "The assistant should cite this injected memory.",
            "decision",
            None,
        )?;
        conn.execute(
            "INSERT INTO context_injection_items
             (injection_run_id, host, project, session_id, injection_key, output_mode,
              decision, item_kind, item_id, memory_id, channel, render_order, status,
              title, provenance, staleness, injected_at_epoch)
             VALUES ('run-1', 'codex-cli', ?1, ?2, 'key-1', 'full',
                     'emitted', 'memory', ?3, ?3, 'core', 1, 'injected',
                     'Usage target', 'src=memory', 'current', 100)",
            params![project, session_id, memory_id],
        )?;
        conn.execute(
            "INSERT INTO summarize_cooldown(project, last_summarize_epoch, last_message_hash)
             VALUES (?1, ?2, NULL)",
            params![project, chrono::Utc::now().timestamp()],
        )?;
        Ok((data_dir, cwd, memory_id))
    }

    fn memory_usage_counts(memory_id: i64) -> Result<(i64, i64, i64)> {
        let conn = db::open_db()?;
        Ok(conn.query_row(
            "SELECT
                 (SELECT COUNT(*) FROM memory_citation_events),
                 (SELECT COUNT(*) FROM memory_usage_events),
                 (SELECT access_count FROM memories WHERE id = ?1)",
            [memory_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?)
    }

    #[test]
    fn raw_archive_status_distinguishes_duplicate_only_from_failed_zero() {
        let duplicate_only = crate::memory::raw_archive::RawIngestReport {
            duplicates: 2,
            ..crate::memory::raw_archive::RawIngestReport::default()
        };
        assert_eq!(
            super::super::side_effects::raw_archive_status(&duplicate_only),
            "duplicate_only"
        );

        let read_failed = crate::memory::raw_archive::RawIngestReport {
            read_error: Some("missing transcript".to_string()),
            ..crate::memory::raw_archive::RawIngestReport::default()
        };
        assert_eq!(
            super::super::side_effects::raw_archive_status(&read_failed),
            "read_failed"
        );
    }

    #[cfg(unix)]
    fn install_summary_stub(path: &std::path::Path) -> Result<()> {
        let script = r#"#!/bin/sh
prev=""
output_path=""
for arg in "$@"; do
  if [ "$prev" = "--output-last-message" ]; then
    output_path="$arg"
    break
  fi
  prev="$arg"
done
if [ -z "$output_path" ]; then
  echo "missing output path" >&2
  exit 1
fi
cat > /dev/null
cat <<'EOF' > "$output_path"
<summary>
  <request>Summary native sync request</request>
  <completed>Summary job kept native memory sync before retirement.</completed>
  <decisions>Keep native memory sync owned before Summary retirement.</decisions>
  <learned></learned>
  <next_steps></next_steps>
  <preferences></preferences>
</summary>
EOF
"#;
        std::fs::write(path, script)?;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
        Ok(())
    }
}
