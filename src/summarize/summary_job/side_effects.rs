use anyhow::Result;

use super::super::input::{extract_last_assistant_message, hash_message, SummarizeInput};

pub(super) fn run_stop_hook_side_effects(
    conn: &rusqlite::Connection,
    host: &str,
    hook: &SummarizeInput,
    session_id: &str,
    project: &str,
    cwd: &str,
    branch: Option<&str>,
) -> Result<String> {
    capture_raw_archive(conn, hook, session_id, project, cwd, branch);
    match crate::memory::failure_lesson::distill_session_failure_lessons(
        conn, session_id, project, branch,
    ) {
        Ok(report) if report.inserted > 0 || report.duplicates > 0 => crate::log::info(
            "summary-job",
            &format!(
                "failure lesson feed inserted={} duplicates={} project={}",
                report.inserted, report.duplicates, project
            ),
        ),
        Ok(_) => {}
        Err(error) => crate::log::error(
            "summary-job",
            &format!(
                "failure lesson feed failed for project={project} session={session_id}: {error}"
            ),
        ),
    }

    let assistant_msg = hook
        .last_assistant_message
        .clone()
        .or_else(|| {
            hook.transcript_path
                .as_deref()
                .and_then(extract_last_assistant_message)
        })
        .unwrap_or_default();
    if !assistant_msg.is_empty() {
        let usage_msg_hash = hash_message(&assistant_msg);
        let usage_report = crate::memory::usage::record_stop_memory_citations(
            conn,
            host,
            project,
            session_id,
            &usage_msg_hash,
            &assistant_msg,
        )?;
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
    Ok(assistant_msg)
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

pub(super) fn raw_archive_status(
    report: &crate::memory::raw_archive::RawIngestReport,
) -> &'static str {
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
