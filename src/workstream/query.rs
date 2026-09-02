use anyhow::Result;
use rusqlite::Connection;

use super::{WorkStream, WorkStreamStatus};

const SELECT_FIELDS: &str =
    "SELECT id, project, title, description, status, progress, next_action, blockers,
                                    created_at_epoch, updated_at_epoch, completed_at_epoch
                             FROM workstreams";

fn workstream_owner_filter(
    conn: &Connection,
    project: &str,
    mut idx: usize,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<(String, usize)> {
    let (owner_clause, next) =
        crate::project_alias::push_project_value_filter(conn, "owner_key", project, idx, params)?;
    idx = next;
    let (target_clause, next) = crate::project_alias::push_project_value_filter(
        conn,
        "target_project",
        project,
        idx,
        params,
    )?;
    idx = next;
    let (legacy_clause, next) =
        crate::project_alias::push_project_value_filter(conn, "project", project, idx, params)?;
    idx = next;
    Ok((
        format!(
            "((owner_scope = 'repo' AND {owner_clause})
               OR (owner_scope = 'repo' AND {target_clause})
               OR (owner_scope = 'workstream' AND {target_clause})
               OR (owner_scope IS NULL AND {legacy_clause}))"
        ),
        idx,
    ))
}

pub fn query_active_workstreams(conn: &Connection, project: &str) -> Result<Vec<WorkStream>> {
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (owner_filter, _) = workstream_owner_filter(conn, project, 1, &mut params_vec)?;
    let sql = format!(
        "{SELECT_FIELDS} WHERE status = 'active'
              AND merged_into_workstream_id IS NULL
              AND {owner_filter}
              ORDER BY updated_at_epoch DESC, id ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params_vec);
    let rows = stmt.query_map(refs.as_slice(), map_workstream_row)?;
    crate::db::query::collect_rows(rows)
}

pub(crate) fn query_active_workstreams_limited(
    conn: &Connection,
    project: &str,
    limit: usize,
) -> Result<Vec<WorkStream>> {
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (owner_filter, idx) = workstream_owner_filter(conn, project, 1, &mut params_vec)?;
    params_vec.push(Box::new(limit as i64));
    let sql = format!(
        "{SELECT_FIELDS} WHERE status = 'active'
              AND merged_into_workstream_id IS NULL
              AND {owner_filter}
              ORDER BY updated_at_epoch DESC, id ASC LIMIT ?{idx}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params_vec);
    let rows = stmt.query_map(refs.as_slice(), map_workstream_row)?;
    crate::db::query::collect_rows(rows)
}

pub fn query_workstreams(
    conn: &Connection,
    project: &str,
    status_filter: Option<&str>,
) -> Result<Vec<WorkStream>> {
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (owner_filter, idx) = workstream_owner_filter(conn, project, 1, &mut params_vec)?;
    let sql = if let Some(status) = status_filter {
        params_vec.push(Box::new(status.to_string()));
        format!(
            "{SELECT_FIELDS} WHERE status = ?{idx}
                  AND merged_into_workstream_id IS NULL
                  AND {owner_filter}
                  ORDER BY updated_at_epoch DESC, id ASC"
        )
    } else {
        format!(
            "{SELECT_FIELDS} WHERE merged_into_workstream_id IS NULL
                  AND {owner_filter}
                  ORDER BY updated_at_epoch DESC, id ASC"
        )
    };

    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params_vec);
    let rows = stmt.query_map(refs.as_slice(), map_workstream_row)?;
    crate::db::query::collect_rows(rows)
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
