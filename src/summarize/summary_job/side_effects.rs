use anyhow::{Context, Result};

use super::super::input::{extract_last_assistant_message, hash_message, SummarizeInput};

pub(super) fn run_stop_hook_side_effects(
    conn: &rusqlite::Connection,
    host: &str,
    hook: &SummarizeInput,
    session_id: &str,
    project: &str,
    cwd: &str,
    branch: Option<&str>,
    drain_raw_archive: bool,
) -> Result<String> {
    if drain_raw_archive {
        capture_raw_archive(conn, hook, session_id, project, cwd, branch);
    }
    if let Err(error) = distill_stop_failure_lessons(conn, session_id, project, branch) {
        crate::log::error(
            "summary-job",
            &format!(
                "failure lesson feed failed for project={project} session={session_id}: {error}"
            ),
        );
    }

    let assistant_msg = hook
        .last_assistant_message
        .clone()
        .or_else(|| {
            drain_raw_archive
                .then(|| {
                    hook.transcript_path
                        .as_deref()
                        .and_then(extract_last_assistant_message)
                })
                .flatten()
        })
        .unwrap_or_default();
    if !assistant_msg.is_empty() {
        if let Err(error) =
            record_stop_memory_citation_usage(conn, host, project, session_id, &assistant_msg)
        {
            crate::log::error(
                "summary-job",
                &format!(
                    "memory citation recording failed for project={project} session={session_id}: {error}"
                ),
            );
        }
    }
    Ok(assistant_msg)
}

pub(crate) fn distill_stop_failure_lessons(
    conn: &rusqlite::Connection,
    session_id: &str,
    project: &str,
    branch: Option<&str>,
) -> Result<()> {
    let report = crate::memory::failure_lesson::distill_session_failure_lessons(
        conn, session_id, project, branch,
    )
    .context("distill Stop-hook failure lessons")?;
    if report.inserted > 0 || report.duplicates > 0 {
        crate::log::info(
            "summary-job",
            &format!(
                "failure lesson feed inserted={} duplicates={} project={}",
                report.inserted, report.duplicates, project
            ),
        );
    }
    Ok(())
}

pub(crate) fn record_stop_memory_citation_usage(
    conn: &rusqlite::Connection,
    host: &str,
    project: &str,
    session_id: &str,
    assistant_msg: &str,
) -> Result<()> {
    let usage_msg_hash = hash_message(assistant_msg);
    let facts = crate::memory::usage::MemoryCitationFacts::from_text(assistant_msg);
    record_stop_memory_citation_evidence(conn, host, project, session_id, &usage_msg_hash, &facts)
}

pub(crate) fn record_stop_memory_citation_evidence(
    conn: &rusqlite::Connection,
    host: &str,
    project: &str,
    session_id: &str,
    message_hash: &str,
    facts: &crate::memory::usage::MemoryCitationFacts,
) -> Result<()> {
    let report = crate::memory::usage::record_stop_memory_citation_facts(
        conn,
        host,
        project,
        session_id,
        message_hash,
        facts,
    )
    .context("record Stop-hook memory citation evidence")?;
    if report.parsed_count > 0 || report.duplicate_event {
        crate::log::info(
            "summary-job",
            &format!(
                "memory citations parsed={} matched={} inserted={} duplicate={} project={}",
                report.parsed_count,
                report.matched_count,
                report.inserted_count,
                report.duplicate_event,
                project
            ),
        );
    }
    Ok(())
}

fn capture_raw_archive(
    conn: &rusqlite::Connection,
    hook: &SummarizeInput,
    session_id: &str,
    project: &str,
    cwd: &str,
    branch: Option<&str>,
) {
    let cwd_opt = Some(cwd);

    if let Some(transcript_path) = hook.transcript_path.as_deref() {
        match crate::memory::raw_archive::drain_transcript(
            conn,
            transcript_path,
            session_id,
            project,
            branch,
            cwd_opt,
        ) {
            Ok(report) => {
                crate::log::info(
                    "summary-job",
                    &format!(
                        "raw archive drained transcript status={} inserted={} duplicates={} parse_errors={} insert_errors={} read_error={} project={}",
                        crate::memory::raw_archive::raw_ingest_status(&report),
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
                        insert_raw_hook_fallback(conn, session_id, project, last, branch, cwd_opt);
                    }
                }
            }
            Err(error) => crate::log::warn(
                "summary-job",
                &format!("raw archive drain failed: {}", error),
            ),
        }
    } else if let Some(last) = hook.last_assistant_message.as_deref() {
        insert_raw_hook_fallback(conn, session_id, project, last, branch, cwd_opt);
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

#[cfg(test)]
mod tests {
    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::super::hook::summarize_input;

    #[tokio::test]
    async fn citation_failure_does_not_block_capture_payload() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("summary-hook-citation-failure");
        let conn = db::open_db()?;
        let now = chrono::Utc::now().timestamp();
        db::upsert_worker_heartbeat(
            &conn,
            "worker-daemon",
            i64::from(std::process::id()),
            now,
            now,
        )?;
        let project = db::project_from_cwd("/tmp/remem");
        let missing_memory_id = 9_999_999_i64;
        conn.execute(
            "INSERT INTO context_injection_items
             (injection_run_id, host, project, session_id, injection_key, output_mode,
              decision, item_kind, item_id, memory_id, channel, render_order, status,
              title, provenance, staleness, injected_at_epoch)
             VALUES ('run-1', 'codex-cli', ?1, 'sess-summary-citation-failure', 'key-1', 'full',
                     'emitted', 'memory', ?2, ?2, 'core', 1, 'injected',
                     'stale memory', 'src=memory', 'current', 100)",
            rusqlite::params![project, missing_memory_id],
        )?;
        drop(conn);
        let input = serde_json::json!({
            "session_id": "sess-summary-citation-failure",
            "cwd": "/tmp/remem",
            "last_assistant_message": format!("assistant cited stale memory\nMemory citations: memory:#{missing_memory_id}")
        })
        .to_string();

        summarize_input(&input, Some("codex-cli"), None).await?;

        let conn = db::open_db()?;
        let job_count: i64 = conn.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))?;
        let captured_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_events
             WHERE session_id = 'sess-summary-citation-failure'
               AND event_type = 'session_stop'",
            [],
            |row| row.get(0),
        )?;
        let summary_jobs: i64 = conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE job_type = 'summary'",
            [],
            |row| row.get(0),
        )?;
        let citation_events: i64 =
            conn.query_row("SELECT COUNT(*) FROM memory_citation_events", [], |row| {
                row.get(0)
            })?;
        let usage_events: i64 =
            conn.query_row("SELECT COUNT(*) FROM memory_usage_events", [], |row| {
                row.get(0)
            })?;

        assert_eq!(job_count, 0);
        assert_eq!(captured_events, 1);
        assert_eq!(summary_jobs, 0);
        assert_eq!(citation_events, 0);
        assert_eq!(usage_events, 0);
        Ok(())
    }
}
