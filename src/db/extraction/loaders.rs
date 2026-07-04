use anyhow::{bail, Result};
use rusqlite::{params, Connection};

use crate::db::ExtractionTaskKind;

use super::ExtractionTask;

pub(super) fn load_claimed_extraction_task(
    conn: &Connection,
    task_id: i64,
) -> Result<ExtractionTask> {
    let row = conn.query_row(
        "SELECT t.id, t.task_kind, t.host_id, t.workspace_id, t.project_id, t.session_row_id,
                h.name, p.project_path, s.session_id,
                t.priority, t.cursor_event_id, t.high_watermark_event_id, t.attempts,
                t.replay_range_id
         FROM extraction_tasks t
         JOIN hosts h ON h.id = t.host_id
         JOIN projects p ON p.id = t.project_id
         LEFT JOIN sessions s ON s.id = t.session_row_id
         WHERE t.id = ?1",
        params![task_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, Option<i64>>(13)?,
            ))
        },
    )?;

    let ai_profile = load_task_ai_profile(conn, row.2, row.4, row.5, row.11)?;
    Ok(ExtractionTask {
        id: row.0,
        task_kind: ExtractionTaskKind::from_db(&row.1)?,
        host_id: row.2,
        workspace_id: row.3,
        project_id: row.4,
        session_row_id: row.5,
        host: row.6,
        project: row.7,
        session_id: row.8,
        ai_profile,
        priority: row.9,
        cursor_event_id: row.10,
        high_watermark_event_id: row.11,
        attempts: row.12,
        replay_range_id: row.13,
    })
}

fn load_task_ai_profile(
    conn: &Connection,
    host_id: i64,
    project_id: i64,
    session_row_id: Option<i64>,
    high_watermark_event_id: Option<i64>,
) -> Result<Option<String>> {
    let Some(session_row_id) = session_row_id else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT COALESCE(
                    CASE
                        WHEN b.content_encoding = 'plain' THEN CAST(b.content_bytes AS TEXT)
                        ELSE NULL
                    END,
                    e.content_text,
                    ''
                ) AS content
         FROM captured_events e
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         WHERE e.host_id = ?1
           AND e.project_id = ?2
           AND e.session_row_id = ?3
           AND (?4 IS NULL OR e.id <= ?4)
         ORDER BY e.id DESC",
    )?;
    let contents = stmt
        .query_map(
            params![host_id, project_id, session_row_id, high_watermark_event_id],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(contents
        .iter()
        .find_map(|content| crate::runtime_config::profile_from_payload_text(content)))
}

pub(super) fn ensure_task_updated(updated: usize, task_id: i64) -> Result<()> {
    if updated == 0 {
        bail!("extraction task {task_id} is not leased by this worker");
    }
    Ok(())
}
