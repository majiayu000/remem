use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;

use crate::db;
use crate::workstream::ParsedWorkStream;

use super::persist::rollup_memory_session_id;
use super::RollupRange;

#[derive(Debug)]
struct PersistedRollupFields {
    request: Option<String>,
    completed: Option<String>,
    decisions: Option<String>,
    learned: Option<String>,
    next_steps: Option<String>,
    preferences: Option<String>,
    summary_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StopHookPayload {
    cwd: Option<String>,
    transcript_path: Option<String>,
    transcript_byte_len: Option<u64>,
    last_assistant_message: Option<String>,
}

pub(super) fn drain_raw_archive_from_range(
    conn: &Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
) -> Result<()> {
    let Some(session_id) = task.session_id.as_deref() else {
        return Ok(());
    };
    let payloads = stop_payloads(range)?;
    if payloads.is_empty() {
        return Ok(());
    }
    let selected_transcripts = unique_transcript_payload_indices(&payloads);
    let mut errors = Vec::new();

    for (payload_index, payload) in payloads.iter().enumerate() {
        let cwd = stop_payload_cwd(payload, &task.project);
        let branch = db::detect_git_branch(cwd);
        let Some(transcript_path) = stop_transcript_path(payload) else {
            if let Err(error) = insert_raw_hook_fallback(
                conn,
                session_id,
                &task.project,
                payload.last_assistant_message.as_deref(),
                branch.as_deref(),
                Some(cwd),
            ) {
                errors.push(error);
            }
            continue;
        };
        if !selected_transcripts.contains(&payload_index) {
            continue;
        }

        let options = crate::memory::raw_archive::TranscriptDrainOptions::default();
        let report = match crate::memory::raw_archive::drain_transcript_with_capture_limit(
            conn,
            transcript_path,
            session_id,
            &task.project,
            branch.as_deref(),
            Some(cwd),
            &options,
            payload.transcript_byte_len,
        ) {
            Ok(report) => report,
            Err(error) => {
                errors.push(error.context("session rollup raw archive drain failed"));
                continue;
            }
        };
        crate::log::info(
            "session-rollup",
            &format!(
                "raw archive drained transcript status={} inserted={} duplicates={} parse_errors={} insert_errors={} read_error={} project={}",
                crate::memory::raw_archive::raw_ingest_status(&report),
                report.inserted,
                report.duplicates,
                report.parse_errors,
                report.insert_errors,
                report.read_error.is_some(),
                task.project
            ),
        );
        if report.read_error.is_some() {
            for fallback in payloads
                .iter()
                .filter(|candidate| stop_transcript_path(candidate) == Some(transcript_path))
            {
                let fallback_cwd = stop_payload_cwd(fallback, &task.project);
                let fallback_branch = db::detect_git_branch(fallback_cwd);
                if let Err(error) = insert_raw_hook_fallback(
                    conn,
                    session_id,
                    &task.project,
                    fallback.last_assistant_message.as_deref(),
                    fallback_branch.as_deref(),
                    Some(fallback_cwd),
                ) {
                    errors.push(error);
                }
            }
        }
        if report.has_failures() {
            errors.push(anyhow::anyhow!(
                "session rollup raw archive ingest incomplete: status={} parse_errors={} insert_errors={} read_error={}",
                crate::memory::raw_archive::raw_ingest_status(&report),
                report.parse_errors,
                report.insert_errors,
                report.read_error.is_some()
            ));
        }
    }

    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.remove(0)),
        _ => anyhow::bail!(
            "session rollup raw archive side effects failed: {}",
            errors
                .iter()
                .map(|error| format!("{error:#}"))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    }
}

pub(super) fn run_post_archive_stop_memory_side_effects(
    conn: &Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
) -> Result<()> {
    let Some(session_id) = task.session_id.as_deref() else {
        return Ok(());
    };
    let payloads = stop_payloads(range)?;
    let Some(latest_payload) = payloads.last() else {
        return Ok(());
    };
    let cwd = stop_payload_cwd(latest_payload, &task.project);
    let branch = db::detect_git_branch(cwd);
    crate::summarize::distill_stop_failure_lessons(
        conn,
        session_id,
        &task.project,
        branch.as_deref(),
    )
    .context("session rollup failure-lesson side effect failed")?;
    for payload in &payloads {
        let assistant_message =
            clean_field(payload.last_assistant_message.as_deref()).or_else(|| {
                payload.transcript_path.as_deref().and_then(|path| {
                    crate::summarize::extract_last_assistant_message_with_limit(
                        path,
                        payload.transcript_byte_len,
                    )
                })
            });
        if let Some(message) = assistant_message {
            crate::summarize::record_stop_memory_citation_usage(
                conn,
                &task.host,
                &task.project,
                session_id,
                &message,
            )
            .context("session rollup memory-citation side effect failed")?;
        }
    }
    Ok(())
}

pub(super) fn run_persisted_rollup_side_effects(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
) -> Result<()> {
    let session_row_id = task
        .session_row_id
        .context("session rollup side effects require session_row_id")?;
    let session_id = task
        .session_id
        .as_deref()
        .context("session rollup side effects require session_id")?;
    let memory_session_id = rollup_memory_session_id(session_row_id);
    let fields = load_persisted_rollup_fields(conn, session_row_id, range)?;
    let cwd = rollup_cwd(task, range);

    link_observed_commits(conn, &task.project, session_id, &memory_session_id)?;
    upsert_rollup_workstream(conn, &task.project, &memory_session_id, &fields)?;
    promote_rollup_candidates(conn, task, range, &fields)?;
    sync_native_memory(conn, &cwd, &task.project)?;
    enqueue_user_context_followup(conn, task, range)?;
    enqueue_summary_followup_jobs(conn, task, session_id)?;
    Ok(())
}

fn rollup_cwd(task: &db::ExtractionTask, range: &RollupRange) -> String {
    latest_stop_payload(range)
        .and_then(|payload| payload.cwd)
        .map(|cwd| cwd.trim().to_string())
        .filter(|cwd| !cwd.is_empty())
        .unwrap_or_else(|| task.project.clone())
}

fn latest_stop_payload(range: &RollupRange) -> Option<StopHookPayload> {
    stop_payloads(range).ok()?.pop()
}

fn stop_payloads(range: &RollupRange) -> Result<Vec<StopHookPayload>> {
    range
        .events
        .iter()
        .filter(|event| event.event_type == "session_stop")
        .filter_map(|event| {
            if !event.content.trim_start().starts_with('{') {
                return None;
            }
            Some(
                serde_json::from_str::<StopHookPayload>(&event.content).with_context(|| {
                    format!(
                        "invalid session_stop payload for captured event {}",
                        event.id
                    )
                }),
            )
        })
        .collect()
}

fn stop_transcript_path(payload: &StopHookPayload) -> Option<&str> {
    payload
        .transcript_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn stop_payload_cwd<'a>(payload: &'a StopHookPayload, project: &'a str) -> &'a str {
    payload
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(project)
}

fn unique_transcript_payload_indices(payloads: &[StopHookPayload]) -> Vec<usize> {
    let mut selected: Vec<(String, usize)> = Vec::new();
    for (index, payload) in payloads.iter().enumerate() {
        let Some(path) = stop_transcript_path(payload) else {
            continue;
        };
        if let Some((_, selected_index)) = selected
            .iter_mut()
            .find(|(selected_path, _)| selected_path == path)
        {
            if prefer_transcript_payload(&payloads[*selected_index], payload) {
                *selected_index = index;
            }
        } else {
            selected.push((path.to_string(), index));
        }
    }
    let mut indices = selected
        .into_iter()
        .map(|(_, index)| index)
        .collect::<Vec<_>>();
    indices.sort_unstable();
    indices
}

fn prefer_transcript_payload(current: &StopHookPayload, candidate: &StopHookPayload) -> bool {
    match (current.transcript_byte_len, candidate.transcript_byte_len) {
        (Some(current), Some(candidate)) => candidate >= current,
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (None, None) => true,
    }
}

fn insert_raw_hook_fallback(
    conn: &Connection,
    session_id: &str,
    project: &str,
    last_message: Option<&str>,
    branch: Option<&str>,
    cwd: Option<&str>,
) -> Result<()> {
    let Some(last) = last_message
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
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
            "session-rollup",
            &format!(
                "raw archive hook fallback inserted={} duplicate={} project={}",
                outcome.inserted, !outcome.inserted, project
            ),
        ),
        Ok(None) => {}
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
                return Err(error).context(format!(
                    "raw archive fallback insert failed and failure recording also failed: {record_error}"
                ));
            }
            return Err(error).context("session rollup raw archive fallback insert failed");
        }
    }
    Ok(())
}

fn load_persisted_rollup_fields(
    conn: &Connection,
    session_row_id: i64,
    range: &RollupRange,
) -> Result<PersistedRollupFields> {
    conn.query_row(
        "SELECT request, completed, decisions, learned, next_steps, preferences, summary_text
         FROM session_summaries
         WHERE session_row_id = ?1
           AND covered_from_event_id = ?2
           AND covered_to_event_id = ?3",
        params![session_row_id, range.from_event_id, range.to_event_id],
        |row| {
            Ok(PersistedRollupFields {
                request: row.get(0)?,
                completed: row.get(1)?,
                decisions: row.get(2)?,
                learned: row.get(3)?,
                next_steps: row.get(4)?,
                preferences: row.get(5)?,
                summary_text: row.get(6)?,
            })
        },
    )
    .optional()?
    .context("persisted session rollup row missing")
}

fn link_observed_commits(
    conn: &Connection,
    project: &str,
    session_id: &str,
    memory_session_id: &str,
) -> Result<()> {
    let linked = crate::git_trace::link_observed_commits_for_session(
        conn,
        project,
        session_id,
        memory_session_id,
    )
    .context("failed to link observed commits for session rollup")?;
    if linked > 0 {
        crate::log::info(
            "session-rollup",
            &format!("linked {linked} observed commit(s) project={project} session={session_id}"),
        );
    }
    Ok(())
}

fn upsert_rollup_workstream(
    conn: &Connection,
    project: &str,
    memory_session_id: &str,
    fields: &PersistedRollupFields,
) -> Result<()> {
    let title = clean_field(fields.request.as_deref());
    let Some(title) = title else {
        return Ok(());
    };
    let parsed = ParsedWorkStream {
        title: Some(title),
        progress: clean_field(fields.completed.as_deref())
            .or_else(|| clean_field(fields.summary_text.as_deref())),
        next_action: clean_field(fields.next_steps.as_deref()),
        blockers: None,
        is_completed: false,
    };
    let result =
        crate::workstream::upsert_workstream_with_match(conn, project, memory_session_id, &parsed)
            .context("session rollup workstream persistence failed")?;
    crate::log::info(
        "session-rollup",
        &format!(
            "upserted workstream id={} reason={} project={project}",
            result.id, result.match_reason
        ),
    );
    Ok(())
}

fn promote_rollup_candidates(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
    fields: &PersistedRollupFields,
) -> Result<()> {
    let session_id = task
        .session_id
        .as_deref()
        .context("session rollup candidate promotion requires session_id")?;
    let evidence_event_ids = range
        .events
        .iter()
        .map(|event| event.id)
        .collect::<Vec<_>>();
    let source_texts = range
        .events
        .iter()
        .map(|event| event.content.trim())
        .filter(|content| !content.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let count = crate::memory::promote::promote_summary_to_memory_candidates_with_evidence(
        conn,
        session_id,
        task.project_id,
        &task.project,
        &evidence_event_ids,
        &source_texts,
        fields.request.as_deref(),
        fields.decisions.as_deref(),
        fields.learned.as_deref(),
        fields.preferences.as_deref(),
    )
    .context("session rollup memory candidate promotion failed")?;
    if count > 0 {
        crate::log::info(
            "session-rollup",
            &format!(
                "promoted {count} summary-derived candidate(s) project={} session={session_id}",
                task.project
            ),
        );
    }
    Ok(())
}

fn sync_native_memory(conn: &Connection, cwd: &str, project: &str) -> Result<()> {
    crate::context::claude_memory::sync_to_claude_memory(conn, cwd, project)
        .context("session rollup native memory sync failed")
}

fn enqueue_user_context_followup(
    conn: &Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
) -> Result<()> {
    db::enqueue_bounded_followup_extraction_task(
        conn,
        task,
        db::ExtractionTaskKind::UserContextCandidate,
        range.from_event_id.saturating_sub(1),
        range.to_event_id,
    )?;
    Ok(())
}

fn enqueue_summary_followup_jobs(
    conn: &Connection,
    task: &db::ExtractionTask,
    session_id: &str,
) -> Result<()> {
    let ready_pending =
        db::count_pending_for_identity(conn, &task.host, &task.project, session_id)?;
    if ready_pending > 0 {
        crate::log::warn(
            "session-rollup",
            &format!(
                "ignored {ready_pending} legacy pending observation row(s); captures now use extraction_tasks"
            ),
        );
    }
    let payload = followup_payload(task.ai_profile.as_deref())?;
    db::enqueue_job(
        conn,
        &task.host,
        db::JobType::Compress,
        &task.project,
        None,
        &payload,
        200,
    )?;
    db::maybe_enqueue_dream_job(
        conn,
        &task.host,
        &task.project,
        &payload,
        300,
        crate::dream::DREAM_COOLDOWN_SECS,
    )?;
    crate::log::info(
        "session-rollup",
        &format!(
            "QUEUED session_rollup_followups session={session_id} project={} legacy_pending_observations={ready_pending}",
            task.project
        ),
    );
    Ok(())
}

fn followup_payload(profile: Option<&str>) -> Result<String> {
    let mut payload = serde_json::Map::new();
    if let Some(profile) = clean_field(profile) {
        payload.insert(
            crate::runtime_config::MEMORY_AI_PROFILE_FIELD.to_string(),
            serde_json::Value::String(profile),
        );
    }
    Ok(serde_json::to_string(&serde_json::Value::Object(payload))?)
}

fn clean_field(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod stop_payload_selection_tests {
    use super::*;

    fn payload(path: &str, transcript_byte_len: Option<u64>) -> StopHookPayload {
        StopHookPayload {
            cwd: None,
            transcript_path: Some(path.to_string()),
            transcript_byte_len,
            last_assistant_message: None,
        }
    }

    #[test]
    fn repeated_transcript_path_selects_one_widest_bounded_payload() {
        let payloads = vec![
            payload("shared.jsonl", Some(10)),
            payload("shared.jsonl", Some(20)),
            payload("shared.jsonl", None),
            payload("legacy.jsonl", None),
        ];

        assert_eq!(unique_transcript_payload_indices(&payloads), vec![1, 3]);
    }
}
