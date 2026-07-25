use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::super::cursor::CursorKind;
use super::super::read_resources::{
    detail_resource, list_resource, redact_bounded, redact_optional, PolicyRelation,
    ReadResourceParams, ReadResourceSpec, ResourceProjectionPolicy, SafeResourceRef,
};
use super::super::types::DbState;

pub(in crate::api) async fn handle_list_workstreams(
    State(_state): State<DbState>,
    Query(params): Query<ReadResourceParams>,
) -> Response {
    list_resource::<Workstreams>(params)
}

pub(in crate::api) async fn handle_workstream_detail(
    State(_state): State<DbState>,
    Path(id): Path<String>,
) -> Response {
    detail_resource::<Workstreams>(id)
}

struct Workstreams;

struct WorkstreamRow {
    id: i64,
    project: String,
    title: String,
    description: Option<String>,
    status: String,
    progress: Option<String>,
    next_action: Option<String>,
    blockers: Option<String>,
    topic_domain: Option<String>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
    completed_at_epoch: Option<i64>,
}

#[derive(Serialize)]
struct WorkstreamItem {
    id: i64,
    project: String,
    title: String,
    description: Option<String>,
    status: String,
    progress: Option<String>,
    next_action: Option<String>,
    blockers: Option<String>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
    completed_at_epoch: Option<i64>,
    references: Vec<SafeResourceRef>,
}

const SELECT_WORKSTREAM: &str =
    "SELECT w.id, w.project, w.title, w.description, w.status, w.progress,
            w.next_action, w.blockers, w.topic_domain, w.created_at_epoch,
            w.updated_at_epoch, w.completed_at_epoch
     FROM workstreams w";

impl ReadResourceSpec for Workstreams {
    type Row = WorkstreamRow;
    type Item = WorkstreamItem;

    const KIND: CursorKind = CursorKind::Workstreams;
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
            "{SELECT_WORKSTREAM}
             WHERE w.merged_into_workstream_id IS NULL
               AND (?1 IS NULL OR w.id < ?1)
               AND (?2 IS NULL OR w.project = ?2)
               AND (?3 IS NULL OR w.status = ?3)
             ORDER BY w.id DESC LIMIT ?4"
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
            &format!(
                "{SELECT_WORKSTREAM}
                 WHERE w.id = ?1 AND w.merged_into_workstream_id IS NULL"
            ),
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
        let mut visible = vec![
            row.project.as_str(),
            row.title.as_str(),
            row.status.as_str(),
        ];
        visible.extend(row.description.as_deref());
        visible.extend(row.progress.as_deref());
        visible.extend(row.next_action.as_deref());
        visible.extend(row.blockers.as_deref());
        visible.extend(row.topic_domain.as_deref());
        let relations = row
            .topic_domain
            .as_deref()
            .map(PolicyRelation::Topic)
            .into_iter()
            .collect::<Vec<_>>();
        if policy.suppresses(&visible, &relations) {
            return Ok(None);
        }
        Ok(Some(WorkstreamItem {
            id: row.id,
            project: redact_bounded(&row.project),
            title: redact_bounded(&row.title),
            description: redact_optional(row.description),
            status: redact_bounded(&row.status),
            progress: redact_optional(row.progress),
            next_action: redact_optional(row.next_action),
            blockers: redact_optional(row.blockers),
            created_at_epoch: row.created_at_epoch,
            updated_at_epoch: row.updated_at_epoch,
            completed_at_epoch: row.completed_at_epoch,
            references: Vec::new(),
        }))
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkstreamRow> {
    Ok(WorkstreamRow {
        id: row.get(0)?,
        project: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        status: row.get(4)?,
        progress: row.get(5)?,
        next_action: row.get(6)?,
        blockers: row.get(7)?,
        topic_domain: row.get(8)?,
        created_at_epoch: row.get(9)?,
        updated_at_epoch: row.get(10)?,
        completed_at_epoch: row.get(11)?,
    })
}
