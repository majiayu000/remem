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

#[derive(Debug, Deserialize, Default)]
pub(in crate::api) struct ActivityParams {
    project: Option<String>,
    source_root: Option<String>,
    session_id: Option<String>,
    before_id: Option<i64>,
    before_epoch: Option<i64>,
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
        params.before_epoch,
        limit,
    ) {
        Ok(data) => Json(json!({
            "meta": { "count": data.len(), "limit": limit },
            "data": data
        }))
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
        Ok(data) => Json(json!({
            "meta": { "count": data.len(), "limit": limit },
            "data": data
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
        Ok(id) => id,
        Err(response) => return response,
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
    if let (Some(since), Some(until)) = (params.since_epoch, params.until_epoch) {
        if since > until {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_activity_window",
                "since_epoch must not exceed until_epoch",
            )
            .into_response();
        }
    }
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    match session_activity::activity_stats(
        &conn,
        trimmed_filter(params.project.as_deref()),
        params.since_epoch,
        params.until_epoch,
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
        Err(error) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "session_activity_projection_failed",
            &error.to_string(),
        )
        .into_response(),
    }
}

fn bounded_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn trimmed_filter(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn parse_positive_id(id: &str) -> Result<i64, Response> {
    id.parse::<i64>().ok().filter(|id| *id > 0).ok_or_else(|| {
        error_response(
            StatusCode::BAD_REQUEST,
            "invalid_session_activity_id",
            "session activity id must be a positive integer",
        )
        .into_response()
    })
}

fn activity_error(code: &str, error: anyhow::Error) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, code, &error.to_string()).into_response()
}
