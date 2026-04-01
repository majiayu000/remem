use anyhow::Result;
use rusqlite::{params, Connection};

use super::{query::map_workstream_row, WorkStream};

pub fn find_matching_workstream(
    conn: &Connection,
    project: &str,
    title: &str,
) -> Result<Option<WorkStream>> {
    let exact = conn
        .query_row(
            "SELECT id, project, title, description, status, progress, next_action, blockers,
                    created_at_epoch, updated_at_epoch, completed_at_epoch
             FROM workstreams
             WHERE project = ?1 AND title = ?2 AND status IN ('active', 'paused')",
            params![project, title],
            map_workstream_row,
        )
        .ok();
    if exact.is_some() {
        return Ok(exact);
    }

    let title_lower = title.to_lowercase();
    let mut stmt = conn.prepare(
        "SELECT id, project, title, description, status, progress, next_action, blockers,
                created_at_epoch, updated_at_epoch, completed_at_epoch
         FROM workstreams
         WHERE project = ?1 AND status IN ('active', 'paused')
         ORDER BY updated_at_epoch DESC",
    )?;
    let rows = stmt.query_map(params![project], map_workstream_row)?;
    for row in rows {
        let workstream = row?;
        let candidate_title = workstream.title.to_lowercase();
        if candidate_title.contains(&title_lower) || title_lower.contains(&candidate_title) {
            return Ok(Some(workstream));
        }
    }

    Ok(None)
}
