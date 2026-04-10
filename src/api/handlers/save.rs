use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use crate::memory_service;

use super::super::helpers::{error_response, open_request_db};
use super::super::types::{DbState, SaveMemoryRequest, SaveMemoryResponse};

pub(in crate::api) async fn handle_save_memory(
    State(_state): State<DbState>,
    Json(req): Json<SaveMemoryRequest>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    let save_req = memory_service::SaveMemoryRequest {
        text: req.text,
        title: req.title,
        project: req.project,
        topic_key: req.topic_key,
        memory_type: req.memory_type,
        files: req.files,
        scope: req.scope,
        created_at_epoch: req.created_at_epoch,
        branch: req.branch,
        local_path: req.local_path,
        local_copy_enabled: req.local_copy_enabled,
    };

    match memory_service::save_memory(&conn, &save_req) {
        Ok(saved) => (
            StatusCode::CREATED,
            Json(SaveMemoryResponse {
                id: saved.id,
                status: saved.status,
                memory_type: saved.memory_type,
                upserted: saved.upserted,
                local_status: saved.local_status,
                local_path: saved.local_path,
            }),
        )
            .into_response(),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "save_failed",
            &err.to_string(),
        )
        .into_response(),
    }
}
