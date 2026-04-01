use anyhow::Result;
use rusqlite::{params, Connection};

use super::{WorkStream, WorkStreamStatus};

const SELECT_FIELDS: &str =
    "SELECT id, project, title, description, status, progress, next_action, blockers,
                                    created_at_epoch, updated_at_epoch, completed_at_epoch
                             FROM workstreams";

pub fn query_active_workstreams(conn: &Connection, project: &str) -> Result<Vec<WorkStream>> {
    let sql = format!(
        "{} WHERE project = ?1 AND status IN ('active', 'paused') ORDER BY updated_at_epoch DESC",
        SELECT_FIELDS
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![project], map_workstream_row)?;
    crate::db_query::collect_rows(rows)
}

pub fn query_workstreams(
    conn: &Connection,
    project: &str,
    status_filter: Option<&str>,
) -> Result<Vec<WorkStream>> {
    let (sql, filter_val) = if let Some(status) = status_filter {
        (
            format!(
                "{} WHERE project = ?1 AND status = ?2 ORDER BY updated_at_epoch DESC",
                SELECT_FIELDS
            ),
            Some(status.to_string()),
        )
    } else {
        (
            format!(
                "{} WHERE project = ?1 ORDER BY updated_at_epoch DESC",
                SELECT_FIELDS
            ),
            None,
        )
    };

    let mut stmt = conn.prepare(&sql)?;
    let rows = if let Some(ref status) = filter_val {
        stmt.query_map(params![project, status], map_workstream_row)?
    } else {
        stmt.query_map(params![project], map_workstream_row)?
    };
    crate::db_query::collect_rows(rows)
}

pub(crate) fn map_workstream_row(row: &rusqlite::Row) -> rusqlite::Result<WorkStream> {
    let status_str: String = row.get(4)?;
    Ok(WorkStream {
        id: row.get(0)?,
        project: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        status: WorkStreamStatus::from_db(&status_str),
        progress: row.get(5)?,
        next_action: row.get(6)?,
        blockers: row.get(7)?,
        created_at_epoch: row.get(8)?,
        updated_at_epoch: row.get(9)?,
        completed_at_epoch: row.get(10)?,
    })
}
