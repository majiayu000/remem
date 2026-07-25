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

pub(in crate::api) async fn handle_list_events(
    State(_state): State<DbState>,
    Query(params): Query<ReadResourceParams>,
) -> Response {
    list_resource::<Events>(params)
}

pub(in crate::api) async fn handle_event_detail(
    State(_state): State<DbState>,
    Path(id): Path<String>,
) -> Response {
    detail_resource::<Events>(id)
}

struct Events;

struct EventRow {
    id: i64,
    project_id: i64,
    project: String,
    session_row_id: i64,
    event_type: String,
    role: Option<String>,
    tool_name: Option<String>,
    retention_class: String,
    created_at_epoch: i64,
    inserted_at_epoch: i64,
    reference_time_epoch: Option<i64>,
}

#[derive(Serialize)]
struct EventItem {
    id: i64,
    project: String,
    event_type: String,
    role: Option<String>,
    tool_name: Option<String>,
    retention_class: String,
    summary: String,
    preview: String,
    created_at_epoch: i64,
    inserted_at_epoch: i64,
    reference_time_epoch: Option<i64>,
    references: Vec<SafeResourceRef>,
}

// Intentionally excludes captured_events.content_text, content_blob_id,
// content_hash, event_id, turn_id, and session_id.
const SELECT_EVENT: &str = "SELECT ce.id, ce.project_id, p.project_key, ce.session_row_id,
            ce.event_type, ce.role, ce.tool_name, ce.retention_class,
            ce.created_at_epoch, ce.inserted_at_epoch, ce.reference_time_epoch
     FROM captured_events ce JOIN projects p ON p.id = ce.project_id";

impl ReadResourceSpec for Events {
    type Row = EventRow;
    type Item = EventItem;

    const KIND: CursorKind = CursorKind::Events;
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
            "{SELECT_EVENT}
             WHERE (?1 IS NULL OR ce.id < ?1)
               AND (?2 IS NULL OR p.project_key = ?2)
               AND (?3 IS NULL OR ce.retention_class = ?3)
             ORDER BY ce.id DESC LIMIT ?4"
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
            &format!("{SELECT_EVENT} WHERE ce.id = ?1"),
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
            row.event_type.as_str(),
            row.retention_class.as_str(),
        ];
        visible.extend(row.role.as_deref());
        visible.extend(row.tool_name.as_deref());
        if policy.suppresses(&visible, &[]) {
            return Ok(None);
        }
        let event_type = safe_event_type(&row.event_type);
        let role = safe_role(row.role.as_deref());
        let tool_name = safe_tool_name(row.tool_name.as_deref());
        let retention_class = safe_retention_class(&row.retention_class);
        let summary = match tool_name.as_deref() {
            Some(tool) => format!("{event_type} event via {tool}"),
            None => format!("{event_type} event"),
        };
        Ok(Some(EventItem {
            id: row.id,
            project: redact_bounded(&row.project),
            event_type,
            role,
            tool_name,
            retention_class,
            summary,
            preview: String::new(),
            created_at_epoch: row.created_at_epoch,
            inserted_at_epoch: row.inserted_at_epoch,
            reference_time_epoch: row.reference_time_epoch,
            references: vec![
                SafeResourceRef {
                    kind: "project",
                    id: row.project_id,
                    title: None,
                    status: None,
                },
                SafeResourceRef {
                    kind: "session",
                    id: row.session_row_id,
                    title: None,
                    status: None,
                },
            ],
        }))
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        project: row.get(2)?,
        session_row_id: row.get(3)?,
        event_type: row.get(4)?,
        role: row.get(5)?,
        tool_name: row.get(6)?,
        retention_class: row.get(7)?,
        created_at_epoch: row.get(8)?,
        inserted_at_epoch: row.get(9)?,
        reference_time_epoch: row.get(10)?,
    })
}

fn safe_event_type(value: &str) -> String {
    match value {
        "file_edit" | "file_create" | "file_write" | "search" | "bash" | "tool_result"
        | "user_prompt_submit" | "session_start" | "session_stop" => value.to_string(),
        _ => "other".to_string(),
    }
}

fn safe_role(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| matches!(*value, "user" | "assistant" | "tool" | "system"))
        .map(str::to_string)
}

fn safe_tool_name(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| {
            matches!(
                *value,
                "Edit" | "Write" | "NotebookEdit" | "Bash" | "Grep" | "Glob" | "Task"
            )
        })
        .map(str::to_string)
}

fn safe_retention_class(value: &str) -> String {
    match value {
        "inline" | "blob" | "metadata" | "ephemeral" | "durable" => value.to_string(),
        _ => "unknown".to_string(),
    }
}
