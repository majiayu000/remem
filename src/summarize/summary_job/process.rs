use std::time::Instant;

use anyhow::Result;

use crate::db;
use crate::db::project_from_cwd;
use crate::perf::{format_phase_timings, push_elapsed, time_result, time_value, PhaseTiming};

use super::super::constants::{
    SUMMARIZE_COOLDOWN_SECS, SUMMARIZE_LOCK_TIMEOUT_SECS, SUMMARY_PROMPT,
};
use super::super::input::{extract_last_assistant_message, hash_message, SummarizeInput};
use super::super::parse::parse_summary;
use super::persist::{build_existing_summary_context, finalize_summary, sync_native_memory};

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

    // Raw archive ingest happens BEFORE every summarize short-circuit so that
    // "what was said is searchable" is independent of curation outcome.
    time_value(&mut timings, "raw_archive", || {
        capture_raw_archive(&conn, &hook, &session_id, &project, cwd)
    });

    let assistant_msg = time_value(&mut timings, "extract_assistant_message", || {
        hook.last_assistant_message
            .clone()
            .or_else(|| {
                hook.transcript_path
                    .as_deref()
                    .and_then(extract_last_assistant_message)
            })
            .unwrap_or_default()
    });
    if !assistant_msg.is_empty() {
        let usage_msg_hash = hash_message(&assistant_msg);
        let usage_report = time_result(&mut timings, "memory_citations", || {
            crate::memory::usage::record_stop_memory_citations(
                &conn,
                host,
                &project,
                &session_id,
                &usage_msg_hash,
                &assistant_msg,
            )
        })?;
        if usage_report.parsed_count > 0 || usage_report.duplicate_event {
            crate::log::info(
                "summary-job",
                &format!(
                    "memory citations parsed={} matched={} inserted={} duplicate={} project={}",
                    usage_report.parsed_count,
                    usage_report.matched_count,
                    usage_report.inserted_count,
                    usage_report.duplicate_event,
                    project
                ),
            );
        }
    }

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

fn capture_raw_archive(
    conn: &rusqlite::Connection,
    hook: &SummarizeInput,
    session_id: &str,
    project: &str,
    cwd: &str,
) {
    let branch = db::detect_git_branch(cwd);
    let cwd_opt = Some(cwd);

    if let Some(transcript_path) = hook.transcript_path.as_deref() {
        match crate::memory::raw_archive::drain_transcript(
            conn,
            transcript_path,
            session_id,
            project,
            branch.as_deref(),
            cwd_opt,
        ) {
            Ok(report) => {
                crate::log::info(
                    "summary-job",
                    &format!(
                        "raw archive drained transcript status={} inserted={} duplicates={} parse_errors={} insert_errors={} read_error={} project={}",
                        raw_archive_status(&report),
                        report.inserted,
                        report.duplicates,
                        report.parse_errors,
                        report.insert_errors,
                        report.read_error.is_some(),
                        project
                    ),
                );
                if report.read_error.is_some() {
                    if let Some(last) = hook.last_assistant_message.as_deref() {
                        insert_raw_hook_fallback(
                            conn,
                            session_id,
                            project,
                            last,
                            branch.as_deref(),
                            cwd_opt,
                        );
                    }
                }
            }
            Err(error) => crate::log::warn(
                "summary-job",
                &format!("raw archive drain failed: {}", error),
            ),
        }
    } else if let Some(last) = hook.last_assistant_message.as_deref() {
        insert_raw_hook_fallback(conn, session_id, project, last, branch.as_deref(), cwd_opt);
    }
}

fn raw_archive_status(report: &crate::memory::raw_archive::RawIngestReport) -> &'static str {
    if report.read_error.is_some() {
        "read_failed"
    } else if report.parse_errors > 0 || report.insert_errors > 0 {
        "partial"
    } else if report.inserted == 0 && report.duplicates > 0 {
        "duplicate_only"
    } else {
        "ok"
    }
}

fn insert_raw_hook_fallback(
    conn: &rusqlite::Connection,
    session_id: &str,
    project: &str,
    last: &str,
    branch: Option<&str>,
    cwd: Option<&str>,
) {
    match crate::memory::raw_archive::insert_raw_message(
        conn,
        session_id,
        project,
        crate::memory::raw_archive::ROLE_ASSISTANT,
        last,
        crate::memory::raw_archive::SOURCE_HOOK,
        branch,
        cwd,
    ) {
        Ok(Some(outcome)) => crate::log::info(
            "summary-job",
            &format!(
                "raw archive hook fallback inserted={} duplicate={} project={}",
                outcome.inserted, !outcome.inserted, project
            ),
        ),
        Ok(None) => crate::log::info(
            "summary-job",
            &format!("raw archive hook fallback empty project={}", project),
        ),
        Err(error) => {
            let report = crate::memory::raw_archive::RawIngestReport {
                insert_errors: 1,
                ..crate::memory::raw_archive::RawIngestReport::default()
            };
            if let Err(record_error) = crate::memory::raw_archive::record_raw_ingest_failure(
                conn,
                session_id,
                project,
                crate::memory::raw_archive::SOURCE_HOOK,
                None,
                &report,
            ) {
                crate::log::warn(
                    "summary-job",
                    &format!("raw archive failure record failed: {}", record_error),
                );
            }
            crate::log::warn(
                "summary-job",
                &format!("raw archive insert failed: {}", error),
            );
        }
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
        assert_eq!(raw_archive_status(&duplicate_only), "duplicate_only");

        let read_failed = crate::memory::raw_archive::RawIngestReport {
            read_error: Some("missing transcript".to_string()),
            ..crate::memory::raw_archive::RawIngestReport::default()
        };
        assert_eq!(raw_archive_status(&read_failed), "read_failed");
    }
}
