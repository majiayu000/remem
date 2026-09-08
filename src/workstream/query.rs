use anyhow::Result;
use rusqlite::Connection;

use super::{WorkStream, WorkStreamStatus};

pub(super) const SELECT_FIELDS: &str =
    "SELECT id, project, title, description, status, progress, next_action, blockers,
            created_at_epoch, updated_at_epoch, completed_at_epoch,
            session_intent, session_topic, session_intent_source
     FROM workstreams";

pub(super) const SELECT_FIELDS_ALIASED: &str =
    "SELECT ws.id, ws.project, ws.title, ws.description, ws.status, ws.progress,
            ws.next_action, ws.blockers, ws.created_at_epoch, ws.updated_at_epoch,
            ws.completed_at_epoch, ws.session_intent, ws.session_topic,
            ws.session_intent_source
     FROM workstreams ws";

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

pub(crate) fn query_active_workstreams_page(
    conn: &Connection,
    project: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<WorkStream>> {
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (owner_filter, idx) = workstream_owner_filter(conn, project, 1, &mut params_vec)?;
    params_vec.push(Box::new(limit as i64));
    let offset_idx = idx + 1;
    params_vec.push(Box::new(offset as i64));
    let sql = format!(
        "{SELECT_FIELDS} WHERE status = 'active'
              AND merged_into_workstream_id IS NULL
              AND {owner_filter}
              ORDER BY updated_at_epoch DESC, id ASC LIMIT ?{idx} OFFSET ?{offset_idx}"
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
    let title: String = row.get(2)?;
    let created_at_epoch: i64 = row.get(8)?;
    let intent: Option<String> = row.get(11)?;
    let topic: Option<String> = row.get(12)?;
    let source: Option<String> = row.get(13)?;
    let label = crate::memory::session_label::render_from_stored(
        Some(created_at_epoch),
        intent.as_deref(),
        topic.as_deref(),
        source.as_deref(),
        Some(&title),
    );
    Ok(WorkStream {
        id: row.get(0)?,
        project: row.get(1)?,
        title,
        description: row.get(3)?,
        status: WorkStreamStatus::from_db(&status_str),
        progress: row.get(5)?,
        next_action: row.get(6)?,
        blockers: row.get(7)?,
        created_at_epoch,
        updated_at_epoch: row.get(9)?,
        completed_at_epoch: row.get(10)?,
        mmdd: label.mmdd,
        session_intent: label.session_intent,
        session_topic: label.session_topic,
        display_label: label.display_label,
        session_intent_source: label.session_intent_source,
    })
}
