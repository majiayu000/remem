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

pub(in crate::api) async fn handle_list_tasks(
    State(_state): State<DbState>,
    Query(params): Query<ReadResourceParams>,
) -> Response {
    list_resource::<Tasks>(params)
}

pub(in crate::api) async fn handle_task_detail(
    State(_state): State<DbState>,
    Path(id): Path<String>,
) -> Response {
    detail_resource::<Tasks>(id)
}

struct Tasks;

struct TaskRow {
    id: i64,
    project_id: i64,
    project: String,
    session_row_id: Option<i64>,
    task_kind: String,
    priority: i64,
    status: String,
    attempts: i64,
    next_retry_epoch: Option<i64>,
    has_error: bool,
    failure_class: Option<String>,
    failed_at_epoch: Option<i64>,
    archived_at_epoch: Option<i64>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
}

#[derive(Serialize)]
struct TaskItem {
    id: i64,
    project: String,
    task_kind: String,
    status: String,
    priority: i64,
    attempts: i64,
    has_error: bool,
    error_class: Option<String>,
    next_retry_epoch: Option<i64>,
    failed_at_epoch: Option<i64>,
    archived_at_epoch: Option<i64>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
    references: Vec<SafeResourceRef>,
}

// last_error is never selected; only its nullness becomes a boolean.
const SELECT_TASK: &str = "SELECT t.id, t.project_id, p.project_key, t.session_row_id,
            t.task_kind, t.priority, t.status, t.attempts, t.next_retry_epoch,
            CASE WHEN t.last_error IS NULL THEN 0 ELSE 1 END,
            t.failure_class, t.failed_at_epoch, t.archived_at_epoch,
            t.created_at_epoch, t.updated_at_epoch
     FROM extraction_tasks t JOIN projects p ON p.id = t.project_id";

impl ReadResourceSpec for Tasks {
    type Row = TaskRow;
    type Item = TaskItem;

    const KIND: CursorKind = CursorKind::Tasks;
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
            "{SELECT_TASK}
             WHERE (?1 IS NULL OR t.id < ?1)
               AND (?2 IS NULL OR p.project_key = ?2)
               AND (?3 IS NULL OR t.status = ?3)
             ORDER BY t.id DESC LIMIT ?4"
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
            &format!("{SELECT_TASK} WHERE t.id = ?1"),
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
            row.task_kind.as_str(),
            row.status.as_str(),
        ];
        visible.extend(row.failure_class.as_deref());
        if policy.suppresses(&visible, &[]) {
            return Ok(None);
        }
        let task_kind = safe_task_kind(&row.task_kind);
        let mut references = vec![SafeResourceRef {
            kind: "project",
            id: row.project_id,
            title: None,
            status: None,
        }];
        references.extend(
            row.session_row_id
                .filter(|id| *id > 0)
                .map(|id| SafeResourceRef {
                    kind: "session",
                    id,
                    title: None,
                    status: None,
                }),
        );
        Ok(Some(TaskItem {
            id: row.id,
            project: redact_bounded(&row.project),
            task_kind,
            status: redact_bounded(&row.status),
            priority: row.priority,
            attempts: row.attempts,
            has_error: row.has_error,
            error_class: row
                .has_error
                .then(|| safe_failure_class(row.failure_class.as_deref())),
            next_retry_epoch: row.next_retry_epoch,
            failed_at_epoch: row.failed_at_epoch,
            archived_at_epoch: row.archived_at_epoch,
            created_at_epoch: row.created_at_epoch,
            updated_at_epoch: row.updated_at_epoch,
            references,
        }))
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        project: row.get(2)?,
        session_row_id: row.get(3)?,
        task_kind: row.get(4)?,
        priority: row.get(5)?,
        status: row.get(6)?,
        attempts: row.get(7)?,
        next_retry_epoch: row.get(8)?,
        has_error: row.get::<_, i64>(9)? != 0,
        failure_class: row.get(10)?,
        failed_at_epoch: row.get(11)?,
        archived_at_epoch: row.get(12)?,
        created_at_epoch: row.get(13)?,
        updated_at_epoch: row.get(14)?,
    })
}

fn safe_task_kind(value: &str) -> String {
    match value {
        "captured_git_link"
        | "session_rollup"
        | "observation_extract"
        | "memory_candidate"
        | "user_context_candidate"
        | "graph_candidate"
        | "rule_candidate"
        | "index_update" => value.to_string(),
        _ => "unknown".to_string(),
    }
}

fn safe_failure_class(value: Option<&str>) -> String {
    match value {
        Some("transient") => "transient",
        Some("permanent") => "permanent",
        Some("manual_reconcile") => "manual_reconcile",
        Some("configuration") => "configuration",
        _ => "unclassified",
    }
    .to_string()
}
