use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;

use crate::db;
use crate::workstream::ParsedWorkStream;

use super::persist::rollup_memory_session_id;
use super::transcript_evidence::{PromptTranscriptMessage, StopCitationEvidence};
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
pub(super) struct StopHookPayload {
    #[serde(skip)]
    pub(super) source_event_id: i64,
    pub(super) cwd: Option<String>,
    pub(super) transcript_path: Option<String>,
    pub(super) transcript_byte_len: Option<u64>,
    pub(super) last_assistant_message: Option<String>,
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
    stop_citations: &[StopCitationEvidence],
    legacy_transcript_messages: &[PromptTranscriptMessage],
    allow_transcript_source_fallback: bool,
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
        if let Some(message) = clean_field(payload.last_assistant_message.as_deref()) {
            crate::summarize::record_stop_memory_citation_usage(
                conn,
                &task.host,
                &task.project,
                session_id,
                &message,
            )
            .context("session rollup memory-citation side effect failed")?;
            continue;
        }
        if let Some(citation) = stop_citations
            .iter()
            .find(|citation| citation.source_event_id == payload.source_event_id)
        {
            crate::summarize::record_stop_memory_citation_evidence(
                conn,
                &task.host,
                &task.project,
                session_id,
                &citation.message_hash,
                &citation.facts,
            )
            .context("session rollup persisted memory-citation side effect failed")?;
            continue;
        }
        if let Some(message) = legacy_transcript_messages
            .iter()
            .rev()
            .find(|message| {
                message.source_event_id == payload.source_event_id && message.role == "assistant"
            })
            .and_then(|message| clean_field(Some(&message.content)))
        {
            crate::summarize::record_stop_memory_citation_usage(
                conn,
                &task.host,
                &task.project,
                session_id,
                &message,
            )
            .context("session rollup legacy v066 memory-citation side effect failed")?;
            continue;
        }
        if allow_transcript_source_fallback {
            let assistant_message = payload.transcript_path.as_deref().and_then(|path| {
                crate::summarize::extract_last_assistant_message_with_limit(
                    path,
                    payload.transcript_byte_len,
                )
            });
            if let Some(message) = assistant_message {
                crate::summarize::record_stop_memory_citation_usage(
                    conn,
                    &task.host,
                    &task.project,
                    session_id,
                    &message,
                )
                .context("session rollup legacy memory-citation side effect failed")?;
            }
        }
    }
    Ok(())
}

pub(super) fn run_persisted_rollup_side_effects(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
    transcript_messages: &[PromptTranscriptMessage],
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

    upsert_rollup_workstream(conn, &task.project, &memory_session_id, &fields)?;
    promote_rollup_candidates(conn, task, range, transcript_messages, &fields)?;
    if let Err(error) = sync_native_memory(conn, &cwd, &task.project) {
        crate::log::error(
            "session-rollup",
            &format!(
                "optional native memory sync failed; continuing persisted rollup side effects: project={} session_row_id={session_row_id} event_range={}..{} error={error:#}",
                task.project, range.from_event_id, range.to_event_id
            ),
        );
    }
    enqueue_user_context_followup(conn, task, range)?;
    enqueue_summary_followup_jobs(conn, task, range, session_id)?;
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

pub(super) fn stop_payloads(range: &RollupRange) -> Result<Vec<StopHookPayload>> {
    range
        .events
        .iter()
        .filter(|event| event.event_type == "session_stop")
        .filter_map(|event| {
            if !event.content.trim_start().starts_with('{') {
                return None;
            }
            Some(
                serde_json::from_str::<StopHookPayload>(&event.content)
                    .map(|mut payload| {
                        payload.source_event_id = event.id;
                        payload
                    })
                    .with_context(|| {
                        format!(
                            "invalid session_stop payload for captured event {}",
                            event.id
                        )
                    }),
            )
        })
        .collect()
}

pub(super) fn stop_transcript_path(payload: &StopHookPayload) -> Option<&str> {
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

pub(super) fn unique_transcript_payload_indices(payloads: &[StopHookPayload]) -> Vec<usize> {
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
    transcript_messages: &[PromptTranscriptMessage],
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
        .chain(
            transcript_messages
                .iter()
                .map(|message| message.content.clone()),
        )
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
    conn: &mut Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
    session_id: &str,
) -> Result<()> {
    let session_row_id = task
        .session_row_id
        .context("session rollup follow-up scheduling requires session_row_id")?;
    let completed_at_epoch = chrono::Utc::now().timestamp();
    let tx = conn.transaction()?;
    let claimed = tx.execute(
        "UPDATE session_summaries
         SET followup_scheduling_state = 'claimed'
         WHERE session_row_id = ?1
           AND covered_from_event_id = ?2
           AND covered_to_event_id = ?3
           AND followup_scheduling_state IS NULL",
        params![session_row_id, range.from_event_id, range.to_event_id],
    )?;
    if claimed == 0 {
        let checkpoint = tx
            .query_row(
                "SELECT followup_scheduling_state,
                        followup_scheduling_completed_at_epoch,
                        followup_compress_job_id,
                        followup_dream_disposition,
                        followup_dream_job_id
                 FROM session_summaries
                 WHERE session_row_id = ?1
                   AND covered_from_event_id = ?2
                   AND covered_to_event_id = ?3",
                params![session_row_id, range.from_event_id, range.to_event_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                },
            )
            .optional()?;
        match checkpoint {
            Some((
                Some(state),
                Some(scheduled_at_epoch),
                Some(compress_job_id),
                Some(dream_disposition),
                Some(dream_job_id),
            )) if state == "completed" => {
                crate::log::info(
                    "session-rollup",
                    &format!(
                        "SKIPPED session_rollup_followups already_scheduled session={session_id} project={} session_row_id={session_row_id} event_range={}..{} scheduled_at_epoch={scheduled_at_epoch} compress_job_id={compress_job_id} dream_disposition={dream_disposition} dream_job_id={dream_job_id}",
                        task.project, range.from_event_id, range.to_event_id,
                    ),
                );
                return Ok(());
            }
            Some((Some(state), None, None, Some(dream_disposition), None))
                if state == "legacy_unknown" && dream_disposition == "legacy_unknown" =>
            {
                crate::log::error(
                    "session-rollup",
                    &format!(
                        "SKIPPED session_rollup_followups manual_reconciliation_required=true session={session_id} project={} session_row_id={session_row_id} event_range={}..{} scheduling_state=legacy_unknown dream_disposition=legacy_unknown",
                        task.project, range.from_event_id, range.to_event_id,
                    ),
                );
                return Ok(());
            }
            Some(record) => anyhow::bail!(
                "persisted session rollup follow-up checkpoint is inconsistent: {record:?}"
            ),
            None => anyhow::bail!("persisted session rollup follow-up checkpoint row is missing"),
        }
    }
    if claimed != 1 {
        anyhow::bail!("persisted session rollup follow-up checkpoint claim matched {claimed} rows");
    }

    let ready_pending = db::count_pending_for_identity(&tx, &task.host, &task.project, session_id)?;
    if ready_pending > 0 {
        crate::log::warn(
            "session-rollup",
            &format!(
                "ignored {ready_pending} legacy pending observation row(s); captures now use extraction_tasks"
            ),
        );
    }
    let payload = followup_payload(task.ai_profile.as_deref())?;
    let compress_job_id = db::enqueue_job_in_transaction(
        &tx,
        &task.host,
        db::JobType::Compress,
        &task.project,
        None,
        &payload,
        200,
    )?;
    let dream_decision = db::maybe_enqueue_dream_job_in_transaction(
        &tx,
        &task.host,
        &task.project,
        &payload,
        300,
        crate::dream::DREAM_COOLDOWN_SECS,
    )?;
    let dream_disposition = dream_decision.disposition();
    let dream_job_id = dream_decision.job_id();
    let completed = tx.execute(
        "UPDATE session_summaries
         SET followup_scheduling_state = 'completed',
             followup_scheduling_completed_at_epoch = ?1,
             followup_compress_job_id = ?2,
             followup_dream_disposition = ?3,
             followup_dream_job_id = ?4
         WHERE session_row_id = ?5
           AND covered_from_event_id = ?6
           AND covered_to_event_id = ?7
           AND followup_scheduling_state = 'claimed'",
        params![
            completed_at_epoch,
            compress_job_id,
            dream_disposition,
            dream_job_id,
            session_row_id,
            range.from_event_id,
            range.to_event_id
        ],
    )?;
    if completed != 1 {
        anyhow::bail!(
            "persisted session rollup follow-up checkpoint completion matched {completed} rows"
        );
    }
    tx.commit()?;
    crate::log::info(
        "session-rollup",
        &format!(
            "COMMITTED session_rollup_followup_decision session={session_id} project={} session_row_id={session_row_id} event_range={}..{} compress_job_id={compress_job_id} dream_disposition={dream_disposition} dream_job_id={dream_job_id} legacy_pending_observations={ready_pending}",
            task.project,
            range.from_event_id,
            range.to_event_id,
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
            source_event_id: 0,
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
