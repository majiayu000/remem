use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rusqlite::{params, OptionalExtension};

use crate::memory_candidate::review::{
    approve_candidate, discard_candidate, edit_candidate, CandidateEdit,
};

use super::super::helpers::{error_response, open_request_db};
use super::super::types::{CandidateEditRequest, CandidateReviewResponse, DbState};

pub(in crate::api) async fn handle_approve_candidate(
    State(_state): State<DbState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let mut conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    match approve_candidate(&mut conn, id) {
        Ok(Some(memory_id)) => Json(CandidateReviewResponse {
            candidate_id: id,
            status: "approved".to_string(),
            memory_id: Some(memory_id),
        })
        .into_response(),
        Ok(None) => candidate_not_found(id),
        Err(err) => candidate_review_error(id, &err),
    }
}

pub(in crate::api) async fn handle_reject_candidate(
    State(_state): State<DbState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    match discard_candidate(&conn, id) {
        Ok(true) => Json(CandidateReviewResponse {
            candidate_id: id,
            status: "discarded".to_string(),
            memory_id: None,
        })
        .into_response(),
        Ok(false) => match candidate_status(&conn, id) {
            Ok(Some(status)) => candidate_not_pending(id, &status),
            Ok(None) => candidate_not_found(id),
            Err(err) => candidate_review_failed(&err),
        },
        Err(err) => candidate_review_failed(&err),
    }
}

pub(in crate::api) async fn handle_edit_candidate(
    State(_state): State<DbState>,
    Path(id): Path<i64>,
    Json(request): Json<CandidateEditRequest>,
) -> impl IntoResponse {
    let edit = match candidate_edit_from_request(request) {
        Ok(edit) => edit,
        Err(message) => {
            return error_response(StatusCode::BAD_REQUEST, "candidate_edit_invalid", &message)
                .into_response()
        }
    };
    let mut conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    match edit_candidate(&mut conn, id, edit) {
        Ok(Some(memory_id)) => Json(CandidateReviewResponse {
            candidate_id: id,
            status: "edited".to_string(),
            memory_id: Some(memory_id),
        })
        .into_response(),
        Ok(None) => candidate_not_found(id),
        Err(err) => candidate_review_error(id, &err),
    }
}

fn candidate_edit_from_request(request: CandidateEditRequest) -> Result<CandidateEdit, String> {
    let text = match request.text {
        Some(text) if text.trim().is_empty() => {
            return Err("edit text must not be empty".to_string())
        }
        other => other,
    };
    if request.scope.is_none()
        && request.memory_type.is_none()
        && request.topic_key.is_none()
        && text.is_none()
    {
        return Err("edit requires at least one changed field".to_string());
    }

    Ok(CandidateEdit {
        scope: request.scope,
        memory_type: request.memory_type,
        topic_key: request.topic_key,
        text,
    })
}

fn candidate_status(conn: &rusqlite::Connection, id: i64) -> anyhow::Result<Option<String>> {
    conn.query_row(
        "SELECT review_status FROM memory_candidates WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn candidate_review_error(id: i64, err: &anyhow::Error) -> axum::response::Response {
    let message = err.to_string();
    if let Some(status) = non_pending_status(&message) {
        return candidate_not_pending(id, status);
    }
    if is_invalid_edit(&message) {
        return error_response(StatusCode::BAD_REQUEST, "candidate_edit_invalid", &message)
            .into_response();
    }
    candidate_review_failed(err)
}

fn non_pending_status(message: &str) -> Option<&str> {
    message
        .strip_prefix("candidate ")
        .and_then(|rest| rest.split_once(" is "))
        .and_then(|(_, rest)| rest.split_once(", expected pending_review"))
        .map(|(status, _)| status)
}

fn is_invalid_edit(message: &str) -> bool {
    message == "edit requires at least one changed field"
        || message.contains("invalid scope")
        || message.contains("invalid memory type")
        || message.contains("empty topic_key")
}

fn candidate_not_found(id: i64) -> axum::response::Response {
    error_response(
        StatusCode::NOT_FOUND,
        "not_found",
        &format!("candidate {id} not found"),
    )
    .into_response()
}

fn candidate_not_pending(id: i64, status: &str) -> axum::response::Response {
    error_response(
        StatusCode::CONFLICT,
        "candidate_not_pending",
        &format!("candidate {id} is already {status}"),
    )
    .into_response()
}

fn candidate_review_failed(err: &anyhow::Error) -> axum::response::Response {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "candidate_review_failed",
        &err.to_string(),
    )
    .into_response()
}
