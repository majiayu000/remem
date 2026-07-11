mod parse;
mod persist;
mod prompt;
mod side_effects;
#[cfg(test)]
mod tests;
mod transcript_evidence;

use std::future::Future;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::db;

const SESSION_ROLLUP_SYSTEM: &str = "\
You summarize captured development-session evidence for a memory system.
Use only the provided events and bounded transcript messages. Preserve concrete
facts, decisions, commands, files, errors, and outcomes. Do not invent missing
details.

Also split the events into coherent topic segments. A topic segment is a set of
events around the same goal, problem, or file area. Use event gap_before,
turn_id, and files_touched hints when present, but choose topic boundaries by
semantic continuity. Return stable kebab-case topic_key values so the same topic
can be linked across sessions.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionRollupResult {
    EmptyRange,
    AlreadyExists,
    Written,
}

#[derive(Debug, Clone)]
pub(super) struct RollupEvent {
    pub(super) id: i64,
    pub(super) event_type: String,
    pub(super) role: Option<String>,
    pub(super) tool_name: Option<String>,
    pub(super) content: String,
    pub(super) token_estimate: i64,
    pub(super) created_at_epoch: i64,
    pub(super) turn_id: Option<String>,
}

pub(super) struct RollupRange {
    pub(super) from_event_id: i64,
    pub(super) to_event_id: i64,
    pub(super) events: Vec<RollupEvent>,
}

pub(crate) async fn process(task: &db::ExtractionTask) -> Result<SessionRollupResult> {
    let mut conn = db::open_db()?;
    let project = task.project.clone();
    let ai_profile = task.ai_profile.clone();
    process_with_summarizer(&mut conn, task, move |prompt| {
        let project = project.clone();
        let ai_profile = ai_profile.clone();
        async move {
            let profile = ai_profile.as_deref();
            crate::ai::call_ai(
                SESSION_ROLLUP_SYSTEM,
                &prompt,
                crate::ai::UsageContext {
                    project: Some(project.as_str()),
                    session_id: task.session_id.as_deref(),
                    operation: "session_rollup",
                    host: profile.is_none().then_some(task.host.as_str()),
                    profile,
                },
            )
            .await
        }
    })
    .await
}

async fn process_with_summarizer<F, Fut>(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    summarize: F,
) -> Result<SessionRollupResult>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let Some(range) = load_rollup_range(conn, task)? else {
        return Ok(SessionRollupResult::EmptyRange);
    };
    if let Some(persisted) = persist::load_persisted_rollup_state(conn, task, &range)? {
        let raw_archive_result = complete_raw_archive_for_existing_rollup(
            conn,
            task,
            &range,
            persisted.raw_archive_completed,
        );
        let side_effect_result = run_rollup_side_effects(
            conn,
            task,
            &range,
            &persisted.transcript_evidence,
            !persisted.has_transcript_evidence_snapshot,
        );
        finish_existing_rollup_retry(raw_archive_result, side_effect_result)?;
        return Ok(SessionRollupResult::AlreadyExists);
    }

    let raw_archive_result = side_effects::drain_raw_archive_from_range(conn, task, &range);
    let transcript_evidence = transcript_evidence::load_prompt_transcript_evidence(&range)?;
    let prompt = prompt::build_rollup_prompt(task, &range, &transcript_evidence);
    let response = summarize(prompt).await?;
    let output = parse::parse_rollup_response(&response, &range)?;
    persist::persist_session_rollup(
        conn,
        task,
        &range,
        &output,
        &transcript_evidence,
        raw_archive_result.is_ok(),
    )?;
    raw_archive_result?;
    run_rollup_side_effects(conn, task, &range, &transcript_evidence, false)?;
    Ok(SessionRollupResult::Written)
}

fn complete_raw_archive_for_existing_rollup(
    conn: &Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
    already_completed: bool,
) -> Result<()> {
    if already_completed {
        return Ok(());
    }
    side_effects::drain_raw_archive_from_range(conn, task, range)?;
    persist::mark_raw_archive_completed(conn, task, range)
}

fn finish_existing_rollup_retry(
    raw_archive_result: Result<()>,
    side_effect_result: Result<()>,
) -> Result<()> {
    match (raw_archive_result, side_effect_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(raw_error), Err(side_error)) => Err(raw_error).context(format!(
            "persisted rollup side effects also failed: {side_error:#}"
        )),
    }
}

fn run_rollup_side_effects(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
    transcript_evidence: &transcript_evidence::PromptTranscriptEvidence,
    allow_transcript_source_fallback: bool,
) -> Result<()> {
    // Early v066 snapshots computed citation hashes from their bounded prompt
    // messages. Replay that exact slice so an upgrade cannot double-count usage.
    let legacy_transcript_messages = if transcript_evidence.citation_evidence_complete {
        &[]
    } else {
        transcript_evidence.messages.as_slice()
    };
    let stop_memory_result = side_effects::run_post_archive_stop_memory_side_effects(
        conn,
        task,
        range,
        &transcript_evidence.stop_citations,
        legacy_transcript_messages,
        allow_transcript_source_fallback,
    );
    let persisted_result = side_effects::run_persisted_rollup_side_effects(
        conn,
        task,
        range,
        &transcript_evidence.messages,
    );
    match (stop_memory_result, persisted_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(stop_error), Err(persisted_error)) => Err(stop_error).context(format!(
            "persisted rollup side effects also failed: {persisted_error:#}"
        )),
    }
}

fn load_rollup_range(conn: &Connection, task: &db::ExtractionTask) -> Result<Option<RollupRange>> {
    let Some(session_row_id) = task.session_row_id else {
        return Ok(None);
    };
    let Some(high_watermark) = task.high_watermark_event_id else {
        return Ok(None);
    };
    let cursor = task.cursor_event_id.unwrap_or(0);
    if high_watermark <= cursor {
        return Ok(None);
    }

    let mut stmt = conn.prepare(
        "SELECT e.id, e.event_type, e.role, e.tool_name,
                COALESCE(
                    CASE
                        WHEN b.content_encoding = 'plain' THEN CAST(b.content_bytes AS TEXT)
                        ELSE NULL
                    END,
                    e.content_text,
                    ''
                ) AS content,
                e.token_estimate, e.created_at_epoch, e.turn_id
         FROM captured_events e
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         WHERE e.host_id = ?1
           AND e.project_id = ?2
           AND e.session_row_id = ?3
           AND e.id > ?4
           AND e.id <= ?5
         ORDER BY e.id ASC",
    )?;
    let events = stmt
        .query_map(
            params![
                task.host_id,
                task.project_id,
                session_row_id,
                cursor,
                high_watermark
            ],
            |row| {
                Ok(RollupEvent {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    role: row.get(2)?,
                    tool_name: row.get(3)?,
                    content: row.get(4)?,
                    token_estimate: row.get(5)?,
                    created_at_epoch: row.get(6)?,
                    turn_id: row.get(7)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;

    if events.is_empty() {
        return Ok(None);
    }
    let from_event_id = events.first().map(|event| event.id).unwrap_or_default();
    let to_event_id = events.last().map(|event| event.id).unwrap_or_default();
    Ok(Some(RollupRange {
        from_event_id,
        to_event_id,
        events,
    }))
}
