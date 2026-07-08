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
    last_assistant_message: Option<String>,
}

pub(super) fn drain_raw_archive_from_range(
    conn: &Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
) {
    let Some(session_id) = task.session_id.as_deref() else {
        return;
    };
    let Some(payload) = latest_stop_payload(range) else {
        return;
    };
    let cwd = payload
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&task.project);
    let branch = db::detect_git_branch(cwd);
    if let Some(transcript_path) = payload
        .transcript_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        match crate::memory::raw_archive::drain_transcript(
            conn,
            transcript_path,
            session_id,
            &task.project,
            branch.as_deref(),
            Some(cwd),
        ) {
            Ok(report) => {
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
                    insert_raw_hook_fallback(
                        conn,
                        session_id,
                        &task.project,
                        payload.last_assistant_message.as_deref(),
                        branch.as_deref(),
                        Some(cwd),
                    );
                }
            }
            Err(error) => crate::log::warn(
                "session-rollup",
                &format!("raw archive drain failed: {error}"),
            ),
        }
    } else {
        insert_raw_hook_fallback(
            conn,
            session_id,
            &task.project,
            payload.last_assistant_message.as_deref(),
            branch.as_deref(),
            Some(cwd),
        );
    }
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
    let fields = load_persisted_rollup_fields(conn, &memory_session_id)?;
    let cwd = rollup_cwd(task, range);

    link_observed_commits(conn, &task.project, session_id, &memory_session_id)?;
    upsert_rollup_workstream(conn, &task.project, &memory_session_id, &fields);
    promote_rollup_candidates(conn, session_id, &task.project, &fields)?;
    sync_native_memory(conn, &cwd, &task.project);
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
    range
        .events
        .iter()
        .rev()
        .find(|event| event.event_type == "session_stop")
        .and_then(|event| serde_json::from_str::<StopHookPayload>(&event.content).ok())
}

fn insert_raw_hook_fallback(
    conn: &Connection,
    session_id: &str,
    project: &str,
    last_message: Option<&str>,
    branch: Option<&str>,
    cwd: Option<&str>,
) {
    let Some(last) = last_message
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
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
        Err(error) => crate::log::warn(
            "session-rollup",
            &format!("raw archive insert failed: {error}"),
        ),
    }
}

fn load_persisted_rollup_fields(
    conn: &Connection,
    memory_session_id: &str,
) -> Result<PersistedRollupFields> {
    conn.query_row(
        "SELECT request, completed, decisions, learned, next_steps, preferences, summary_text
         FROM session_summaries
         WHERE memory_session_id = ?1",
        params![memory_session_id],
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
) {
    let title = clean_field(fields.request.as_deref());
    let Some(title) = title else {
        return;
    };
    let parsed = ParsedWorkStream {
        title: Some(title),
        progress: clean_field(fields.completed.as_deref())
            .or_else(|| clean_field(fields.summary_text.as_deref())),
        next_action: clean_field(fields.next_steps.as_deref()),
        blockers: None,
        is_completed: false,
    };
    match crate::workstream::upsert_workstream_with_match(conn, project, memory_session_id, &parsed)
    {
        Ok(result) => crate::log::info(
            "session-rollup",
            &format!(
                "upserted workstream id={} reason={} project={project}",
                result.id, result.match_reason
            ),
        ),
        Err(error) => crate::log::warn(
            "session-rollup",
            &format!("workstream persistence failed: {error}"),
        ),
    }
}

fn promote_rollup_candidates(
    conn: &mut Connection,
    session_id: &str,
    project: &str,
    fields: &PersistedRollupFields,
) -> Result<()> {
    let count = crate::memory::promote_summary_to_memory_candidates(
        conn,
        session_id,
        project,
        fields.request.as_deref(),
        fields.decisions.as_deref(),
        fields.learned.as_deref(),
        fields.preferences.as_deref(),
    )
    .context("session rollup memory candidate promotion failed")?;
    if count > 0 {
        crate::log::info(
            "session-rollup",
            &format!("promoted {count} summary-derived candidate(s) project={project} session={session_id}"),
        );
    }
    Ok(())
}

fn sync_native_memory(conn: &Connection, cwd: &str, project: &str) {
    if let Err(error) = crate::context::claude_memory::sync_to_claude_memory(conn, cwd, project) {
        crate::log::warn(
            "session-rollup",
            &format!("claude memory sync failed: {error}"),
        );
    }
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
