use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::super::cursor::CursorKind;
use super::super::read_resources::{
    detail_resource, list_resource, redact_bounded, ReadResourceParams, ReadResourceSpec,
    ResourceProjectionPolicy, SafeResourceRef,
};
use super::super::types::DbState;

pub(in crate::api) async fn handle_list_sessions(
    State(_state): State<DbState>,
    Query(params): Query<ReadResourceParams>,
) -> Response {
    list_resource::<Sessions>(params)
}

pub(in crate::api) async fn handle_session_detail(
    State(_state): State<DbState>,
    Path(id): Path<String>,
) -> Response {
    detail_resource::<Sessions>(id)
}

struct Sessions;

struct SessionRow {
    id: i64,
    host_id: i64,
    host: String,
    project_id: i64,
    project: String,
    started_at_epoch: Option<i64>,
    last_seen_at_epoch: i64,
    status: String,
}

#[derive(Serialize)]
struct SessionItem {
    id: i64,
    host: String,
    project: String,
    status: String,
    summary: String,
    started_at_epoch: Option<i64>,
    last_seen_at_epoch: i64,
    references: Vec<SafeResourceRef>,
}

const SELECT_SESSION: &str = "SELECT s.id, s.host_id, h.name, s.project_id, p.project_key,
            s.started_at_epoch, s.last_seen_at_epoch, s.status
     FROM sessions s
     JOIN hosts h ON h.id = s.host_id
     JOIN projects p ON p.id = s.project_id";

impl ReadResourceSpec for Sessions {
    type Row = SessionRow;
    type Item = SessionItem;

    const KIND: CursorKind = CursorKind::Sessions;
    fn row_id(row: &Self::Row) -> i64 {
        row.id
    }

    fn load_batch(
        conn: &Connection,
        resume_before_id: Option<i64>,
        project: Option<&str>,
        status: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<Self::Row>> {
        let sql = format!(
            "{SELECT_SESSION}
             WHERE (?1 IS NULL OR s.id < ?1)
               AND (?2 IS NULL OR p.project_key = ?2)
               AND (?3 IS NULL OR s.status = ?3)
             ORDER BY s.id DESC LIMIT ?4"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![resume_before_id, project, status, limit as i64],
            map_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn load_one(conn: &Connection, id: i64) -> anyhow::Result<Option<Self::Row>> {
        conn.query_row(
            &format!("{SELECT_SESSION} WHERE s.id = ?1"),
            params![id],
            map_row,
        )
        .optional()
        .map_err(Into::into)
    }

    fn project(
        row: Self::Row,
        policy: &ResourceProjectionPolicy,
    ) -> anyhow::Result<Option<Self::Item>> {
        if policy.suppresses(&[&row.host, &row.project, &row.status], &[]) {
            return Ok(None);
        }
        let host = redact_bounded(&row.host);
        let project = redact_bounded(&row.project);
        let status = redact_bounded(&row.status);
        Ok(Some(SessionItem {
            id: row.id,
            summary: format!("Session on {host}"),
            host: host.clone(),
            project: project.clone(),
            status,
            started_at_epoch: row.started_at_epoch,
            last_seen_at_epoch: row.last_seen_at_epoch,
            references: vec![
                SafeResourceRef {
                    kind: "host",
                    id: row.host_id,
                    title: Some(host),
                    status: None,
                },
                SafeResourceRef {
                    kind: "project",
                    id: row.project_id,
                    title: Some(project),
                    status: None,
                },
            ],
        }))
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        host_id: row.get(1)?,
        host: row.get(2)?,
        project_id: row.get(3)?,
        project: row.get(4)?,
        started_at_epoch: row.get(5)?,
        last_seen_at_epoch: row.get(6)?,
        status: row.get(7)?,
    })
}
