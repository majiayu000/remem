use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db;

use super::parse::RollupOutput;
use super::transcript_evidence::PromptTranscriptEvidence;
use super::RollupRange;

pub(super) struct PersistedRollupState {
    pub(super) transcript_evidence: PromptTranscriptEvidence,
    pub(super) has_transcript_evidence_snapshot: bool,
    pub(super) raw_archive_completed: bool,
}

pub(super) fn persist_session_rollup(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
    output: &RollupOutput,
    transcript_evidence: &PromptTranscriptEvidence,
    raw_archive_completed: bool,
) -> Result<()> {
    let session_row_id = task
        .session_row_id
        .context("session_rollup task missing session_row_id")?;
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let created_at_epoch = now.timestamp();
    let memory_session_id = rollup_memory_session_id(session_row_id);
    let fallback_request = format!(
        "Captured event range {}..{}",
        range.from_event_id, range.to_event_id
    );
    let request = output
        .structured_fields
        .request
        .as_deref()
        .unwrap_or(&fallback_request);
    let discovery_tokens = estimate_discovery_tokens(output);
    ensure!(
        transcript_evidence.citation_evidence_complete,
        "invalid payload: new session rollup is missing complete Stop citation evidence"
    );
    transcript_evidence.validate_for_range(range)?;
    let transcript_evidence_json = serde_json::to_string(transcript_evidence)
        .context("serialize bounded transcript evidence for session rollup")?;
    let raw_archive_completed_at_epoch = raw_archive_completed.then_some(created_at_epoch);
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at, created_at_epoch,
          decisions, learned, next_steps, preferences, discovery_tokens,
          host_id, project_id, session_row_id, summary_text,
          covered_from_event_id, covered_to_event_id, model,
          transcript_evidence_json, raw_archive_completed_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, NULL, ?18, ?19)",
        params![
            memory_session_id,
            task.project,
            request,
            output.summary_text,
            created_at,
            created_at_epoch,
            output.structured_fields.decisions.as_deref(),
            output.structured_fields.learned.as_deref(),
            output.structured_fields.next_steps.as_deref(),
            output.structured_fields.preferences.as_deref(),
            discovery_tokens,
            task.host_id,
            task.project_id,
            session_row_id,
            output.summary_text,
            range.from_event_id,
            range.to_event_id,
            transcript_evidence_json,
            raw_archive_completed_at_epoch
        ],
    )?;

    for segment in &output.segments {
        let evidence_json = serde_json::to_string(&segment.evidence_event_ids)?;
        let files_json = if segment.files.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&segment.files)?)
        };
        db::insert_topic_segment(
            &tx,
            &db::TopicSegmentInput {
                host_id: task.host_id,
                project_id: task.project_id,
                session_row_id,
                project: &task.project,
                topic_key: &segment.topic_key,
                title: &segment.title,
                summary: &segment.summary,
                status: &segment.status,
                segment_index: segment.segment_index,
                covered_from_event_id: segment.covered_from_event_id,
                covered_to_event_id: segment.covered_to_event_id,
                evidence_event_ids: &evidence_json,
                files: files_json.as_deref(),
                confidence: segment.confidence,
            },
        )?;
    }

    tx.commit()?;
    Ok(())
}

pub(super) fn load_persisted_rollup_state(
    conn: &Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
) -> Result<Option<PersistedRollupState>> {
    let Some(session_row_id) = task.session_row_id else {
        return Ok(None);
    };
    let row = conn
        .query_row(
            "SELECT transcript_evidence_json, raw_archive_completed_at_epoch
             FROM session_summaries
             WHERE session_row_id = ?1
               AND covered_from_event_id = ?2
               AND covered_to_event_id = ?3
             LIMIT 1",
            params![session_row_id, range.from_event_id, range.to_event_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                ))
            },
        )
        .optional()?;
    let Some((evidence_json, raw_archive_completed_at_epoch)) = row else {
        return Ok(None);
    };
    let (transcript_evidence, has_transcript_evidence_snapshot) = match evidence_json {
        Some(json) => (
            serde_json::from_str::<PromptTranscriptEvidence>(&json)
                .context("parse persisted bounded transcript evidence for session rollup")?,
            true,
        ),
        None => {
            crate::log::info(
                "session-rollup",
                "legacy persisted rollup has no transcript evidence snapshot; retrying with bounded source fallback enabled",
            );
            (PromptTranscriptEvidence::default(), false)
        }
    };
    transcript_evidence.validate_for_range(range)?;
    Ok(Some(PersistedRollupState {
        transcript_evidence,
        has_transcript_evidence_snapshot,
        raw_archive_completed: raw_archive_completed_at_epoch.is_some(),
    }))
}

pub(super) fn mark_raw_archive_completed(
    conn: &Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
) -> Result<()> {
    let session_row_id = task
        .session_row_id
        .context("session_rollup task missing session_row_id")?;
    let updated = conn.execute(
        "UPDATE session_summaries
         SET raw_archive_completed_at_epoch = COALESCE(raw_archive_completed_at_epoch, ?1)
         WHERE session_row_id = ?2
           AND covered_from_event_id = ?3
           AND covered_to_event_id = ?4",
        params![
            chrono::Utc::now().timestamp(),
            session_row_id,
            range.from_event_id,
            range.to_event_id
        ],
    )?;
    if updated != 1 {
        anyhow::bail!(
            "persisted session rollup raw archive checkpoint update matched {updated} rows"
        );
    }
    Ok(())
}

pub(super) fn rollup_memory_session_id(session_row_id: i64) -> String {
    format!("capture-rollup-{session_row_id}")
}

fn estimate_discovery_tokens(output: &RollupOutput) -> i64 {
    let structured_len = [
        Some(output.summary_text.as_str()),
        output.structured_fields.request.as_deref(),
        output.structured_fields.decisions.as_deref(),
        output.structured_fields.learned.as_deref(),
        output.structured_fields.next_steps.as_deref(),
        output.structured_fields.preferences.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::len)
    .sum::<usize>() as i64;
    (structured_len + 3) / 4
}
