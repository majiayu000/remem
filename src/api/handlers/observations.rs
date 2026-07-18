use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::super::cursor::CursorKind;
use super::super::read_resources::{
    detail_resource, list_resource, redact_bounded, redact_optional, ReadResourceParams,
    ReadResourceSpec, ResourceProjectionPolicy, SafeResourceRef,
};
use super::super::types::DbState;

pub(in crate::api) async fn handle_list_observations(
    State(_state): State<DbState>,
    Query(params): Query<ReadResourceParams>,
) -> Response {
    list_resource::<Observations>(params)
}

pub(in crate::api) async fn handle_observation_detail(
    State(_state): State<DbState>,
    Path(id): Path<String>,
) -> Response {
    detail_resource::<Observations>(id)
}

struct Observations;

struct ObservationRow {
    id: i64,
    project: Option<String>,
    observation_type: String,
    title: Option<String>,
    status: String,
    branch: Option<String>,
    created_at_epoch: Option<i64>,
    reference_time_epoch: Option<i64>,
    session_row_id: Option<i64>,
}

#[derive(Serialize)]
struct ObservationItem {
    id: i64,
    project: Option<String>,
    observation_type: String,
    status: String,
    title: Option<String>,
    summary: String,
    preview: String,
    created_at_epoch: Option<i64>,
    reference_time_epoch: Option<i64>,
    branch: Option<String>,
    references: Vec<SafeResourceRef>,
}

const SELECT_OBSERVATION: &str = "SELECT o.id, COALESCE(o.project, p.project_key),
            COALESCE(o.observation_type, o.type), o.title,
            COALESCE(o.status, 'active'), o.branch, o.created_at_epoch,
            o.reference_time_epoch, o.session_row_id
     FROM observations o LEFT JOIN projects p ON p.id = o.project_id";

impl ReadResourceSpec for Observations {
    type Row = ObservationRow;
    type Item = ObservationItem;

    const KIND: CursorKind = CursorKind::Observations;
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
            "{SELECT_OBSERVATION}
             WHERE (?1 IS NULL OR o.id < ?1)
               AND (?2 IS NULL OR COALESCE(o.project, p.project_key) = ?2)
               AND (?3 IS NULL OR COALESCE(o.status, 'active') = ?3)
             ORDER BY o.id DESC LIMIT ?4"
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
            &format!("{SELECT_OBSERVATION} WHERE o.id = ?1"),
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
        let mut visible = vec![row.observation_type.as_str(), row.status.as_str()];
        visible.extend(row.project.as_deref());
        visible.extend(row.title.as_deref());
        visible.extend(row.branch.as_deref());
        if policy.suppresses(&visible, &[]) {
            return Ok(None);
        }
        let observation_type = redact_bounded(&row.observation_type);
        let title = redact_optional(row.title);
        let summary = title
            .clone()
            .unwrap_or_else(|| format!("Observation {observation_type}"));
        let preview = title.clone().unwrap_or_default();
        let references = row
            .session_row_id
            .filter(|id| *id > 0)
            .map(|id| SafeResourceRef {
                kind: "session",
                id,
                title: None,
                status: None,
            })
            .into_iter()
            .collect();
        Ok(Some(ObservationItem {
            id: row.id,
            project: redact_optional(row.project),
            observation_type,
            status: redact_bounded(&row.status),
            title,
            summary,
            preview,
            created_at_epoch: row.created_at_epoch,
            reference_time_epoch: row.reference_time_epoch,
            branch: redact_optional(row.branch),
            references,
        }))
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ObservationRow> {
    Ok(ObservationRow {
        id: row.get(0)?,
        project: row.get(1)?,
        observation_type: row.get(2)?,
        title: row.get(3)?,
        status: row.get(4)?,
        branch: row.get(5)?,
        created_at_epoch: row.get(6)?,
        reference_time_epoch: row.get(7)?,
        session_row_id: row.get(8)?,
    })
}
