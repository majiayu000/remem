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
    let memory_session_id = format!("capture-rollup-{session_row_id}");
    let request = format!(
        "Captured event range {}..{}",
        range.from_event_id, range.to_event_id
    );
    let discovery_tokens = ((output.summary_text.len() as i64) + 3) / 4;
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at, created_at_epoch,
          discovery_tokens, host_id, project_id, session_row_id, summary_text,
          covered_from_event_id, covered_to_event_id, model)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL)",
        params![
            memory_session_id,
            task.project,
            request,
            output.summary_text,
            created_at,
            created_at_epoch,
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
