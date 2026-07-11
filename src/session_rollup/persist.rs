use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::db;

use super::parse::RollupOutput;
use super::RollupRange;

pub(super) fn persist_session_rollup(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    range: &RollupRange,
    output: &RollupOutput,
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
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at, created_at_epoch,
          decisions, learned, next_steps, preferences, discovery_tokens,
          host_id, project_id, session_row_id, summary_text,
          covered_from_event_id, covered_to_event_id, model)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, NULL)",
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
            range.to_event_id
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

pub(crate) fn rollup_memory_session_id(session_row_id: i64) -> String {
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
