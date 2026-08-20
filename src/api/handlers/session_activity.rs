use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::session_activity::{self, SessionActivityKey};

use super::super::helpers::{error_response, open_request_db};
use super::super::types::DbState;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const DEFAULT_STATS_WINDOW_SECS: i64 = 30 * 24 * 60 * 60;
const MAX_STATS_WINDOW_SECS: i64 = 366 * 24 * 60 * 60;

#[derive(Debug, Deserialize, Default)]
pub(in crate::api) struct ActivityParams {
    project: Option<String>,
    source_root: Option<String>,
    session_id: Option<String>,
    before_id: Option<i64>,
    cursor: Option<String>,
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct ProjectActivityRequest {
    source_root: String,
    project: String,
    session_id: String,
}

pub(in crate::api) async fn handle_activity_sessions(
    State(_state): State<DbState>,
    Query(params): Query<ActivityParams>,
) -> Response {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    let limit = bounded_limit(params.limit);
    match session_activity::list_activity_sessions(
        &conn,
        trimmed_filter(params.project.as_deref()),
        trimmed_filter(params.cursor.as_deref()),
        limit,
    ) {
        Ok(page) => Json(json!({
            "meta": {
                "count": page.data.len(),
                "limit": limit,
                "has_more": page.has_more,
                "next_cursor": page.next_cursor
            },
            "data": page.data
        }))
        .into_response(),
        Err(error) if error.to_string().contains("cursor") => error_response(
            StatusCode::BAD_REQUEST,
            "invalid_session_activity_cursor",
            "session activity cursor is invalid or does not match the requested filters",
        )
        .into_response(),
        Err(error) => activity_error("session_activity_sessions_failed", error),
    }
}

pub(in crate::api) async fn handle_list_session_activity(
    State(_state): State<DbState>,
    Query(params): Query<ActivityParams>,
) -> Response {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    let limit = bounded_limit(params.limit);
    match session_activity::list_turns(
        &conn,
        trimmed_filter(params.project.as_deref()),
        trimmed_filter(params.source_root.as_deref()),
        trimmed_filter(params.session_id.as_deref()),
        params.before_id,
        limit,
    ) {
        Ok(page) => Json(json!({
            "meta": {
                "count": page.data.len(),
                "limit": limit,
                "has_more": page.has_more,
                "next_before_id": page.next_before_id
            },
            "data": page.data
        }))
        .into_response(),
        Err(error) => activity_error("session_activity_list_failed", error),
    }
}

pub(in crate::api) async fn handle_session_activity_detail(
    State(_state): State<DbState>,
    Path(id): Path<String>,
) -> Response {
    let id = match parse_positive_id(&id) {
        Some(id) => id,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_session_activity_id",
                "session activity id must be a positive integer",
            )
            .into_response()
        }
    };
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    match session_activity::get_turn(&conn, id) {
        Ok(Some(turn)) => Json(json!({ "data": turn })).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "session_activity_not_found",
            "session activity turn not found",
        )
        .into_response(),
        Err(error) => activity_error("session_activity_detail_failed", error),
    }
}

pub(in crate::api) async fn handle_session_activity_stats(
    State(_state): State<DbState>,
    Query(params): Query<ActivityParams>,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let until = params.until_epoch.unwrap_or(now);
    let since = params
        .since_epoch
        .unwrap_or_else(|| until.saturating_sub(DEFAULT_STATS_WINDOW_SECS));
    if since > until || until.saturating_sub(since) > MAX_STATS_WINDOW_SECS {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_activity_window",
            "activity window must be ordered and no wider than 366 days",
        )
        .into_response();
    }
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    match session_activity::activity_stats(
        &conn,
        trimmed_filter(params.project.as_deref()),
        Some(since),
        Some(until),
    ) {
        Ok(data) => Json(json!({ "data": data })).into_response(),
        Err(error) => activity_error("session_activity_stats_failed", error),
    }
}

pub(in crate::api) async fn handle_project_session_activity(
    State(_state): State<DbState>,
    Json(request): Json<ProjectActivityRequest>,
) -> Response {
    let key = SessionActivityKey {
        source_root: request.source_root,
        project: request.project,
        session_id: request.session_id,
    };
    let mut conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    match session_activity::project_session(&mut conn, &key, chrono::Utc::now().timestamp()) {
        Ok(data) => Json(json!({ "data": data })).into_response(),
        Err(error) => {
            crate::log::error(
                "api",
                &format!("session activity projection failed: {error:#}"),
            );
            let redacted = crate::adapter::common::redact_sensitive_text(&error.to_string());
            let detail = crate::db::truncate_str(&redacted, 500);
            error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "session_activity_projection_failed",
                detail,
            )
            .into_response()
        }
    }
}

fn bounded_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn trimmed_filter(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn parse_positive_id(id: &str) -> Option<i64> {
    id.parse::<i64>().ok().filter(|id| *id > 0)
}

fn activity_error(code: &str, error: anyhow::Error) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, code, &error.to_string()).into_response()
}
