use std::collections::BTreeMap;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::db;
use crate::git_util::GitCommitMetadata;

pub(crate) fn link_task_range(
    conn: &mut Connection,
    task: &db::ExtractionTask,
) -> Result<Vec<GitCommitMetadata>> {
    let session_row_id = task
        .session_row_id
        .context("captured commit link requires session_row_id")?;
    let high_watermark = task
        .high_watermark_event_id
        .context("captured commit link requires high_watermark_event_id")?;
    let cursor = task.cursor_event_id.unwrap_or(0);
    if high_watermark <= cursor {
        return Ok(Vec::new());
    }
    let range_label = format!(
        "session_row_id={session_row_id} range={}..{}",
        cursor + 1,
        high_watermark
    );
    let mut stmt = conn.prepare(
        "SELECT events.id, evidence.sha, evidence.metadata_json
         FROM captured_events events
         JOIN captured_event_commits evidence ON evidence.event_row_id = events.id
         WHERE events.host_id = ?1
           AND events.project_id = ?2
           AND events.session_row_id = ?3
           AND events.id > ?4
           AND events.id <= ?5
           AND evidence.evidence_kind = 'observed_commit'
         ORDER BY events.id ASC, evidence.sha ASC",
    )?;
    let rows = stmt
        .query_map(
            params![
                task.host_id,
                task.project_id,
                session_row_id,
                cursor,
                high_watermark
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()
        .with_context(|| format!("load captured Git evidence {range_label}"))?;
    drop(stmt);

    let mut commits = BTreeMap::new();
    for (event_id, sha, raw_metadata) in rows {
        let metadata: GitCommitMetadata =
            serde_json::from_str(&raw_metadata).with_context(|| {
                format!(
                    "invalid captured Git metadata JSON {range_label} event={event_id} sha={sha}"
                )
            })?;
        if !metadata.sha.trim().eq_ignore_ascii_case(sha.trim()) {
            anyhow::bail!(
                "captured Git metadata mismatch {range_label} event={event_id}: column sha={sha} metadata sha={}",
                metadata.sha
            );
        }
        commits.insert(sha.trim().to_ascii_lowercase(), metadata);
    }
    let commits = commits.into_values().collect::<Vec<_>>();
    if commits.is_empty() {
        return Ok(commits);
    }

    let session_id = task
        .session_id
        .as_deref()
        .context("captured commit link requires session_id")?;
    let memory_session_id = crate::session_rollup::rollup_memory_session_id(session_row_id);
    let tx = conn.transaction()?;
    for metadata in &commits {
        crate::git_trace::link_captured_git_metadata_to_session(
            &tx,
            &task.project,
            session_row_id,
            session_id,
            &memory_session_id,
            metadata,
        )
        .with_context(|| {
            format!(
                "captured commit link failed: session_row_id={session_row_id} range={}..{} sha={}",
                cursor + 1,
                high_watermark,
                metadata.sha
            )
        })?;
    }
    tx.commit()?;
    crate::log::info(
        "captured-git",
        &format!(
            "linked {} captured commit(s) session_row_id={} range={}..{} task={}",
            commits.len(),
            session_row_id,
            cursor + 1,
            high_watermark,
            task.task_kind.as_str()
        ),
    );
    Ok(commits)
}
