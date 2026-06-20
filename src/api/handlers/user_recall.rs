use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use super::super::helpers::{error_response, open_request_db};
use super::super::types::{DbState, UserRecallRequest};

pub(in crate::api) async fn handle_user_recall(
    State(_state): State<DbState>,
    Json(params): Json<UserRecallRequest>,
) -> impl IntoResponse {
    if params.query.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_recall_request",
            "query is required",
        )
        .into_response();
    }

    let project = match resolve_recall_project(params.project.as_deref(), params.cwd.as_deref()) {
        Ok(project) => project,
        Err(message) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_user_recall_request",
                &message,
            )
            .into_response()
        }
    };
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    match crate::user_context::recall::recall_user_context(
        &conn,
        &crate::user_context::recall::UserRecallRequest {
            query: params.query,
            project,
            task_intent: params.task_intent,
            current_files: params.current_files,
            host: params.host,
            owner_scope: params.owner_scope,
            owner_key: params.owner_key,
            state_keys: params.state_keys,
            include_sensitive: params.include_sensitive,
            include_suppressed: params.include_suppressed,
            limit: params.limit,
            budget_chars: params.budget_chars,
        },
    ) {
        Ok(result) => Json(result).into_response(),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "user_recall_failed",
            &err.to_string(),
        )
        .into_response(),
    }
}

fn resolve_recall_project(project: Option<&str>, cwd: Option<&str>) -> Result<String, String> {
    if let Some(project) = project.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(project.to_string());
    }
    if let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(crate::db::project_from_cwd(cwd));
    }
    Err("project or cwd is required".to_string())
}
