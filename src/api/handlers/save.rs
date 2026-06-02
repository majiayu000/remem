use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use crate::memory::service;

use super::super::helpers::{error_response, open_request_db};
use super::super::types::{
    DbState, LocalCopyResponse, SaveMemoryNextStepResponse, SaveMemoryRequest, SaveMemoryResponse,
};

pub(in crate::api) async fn handle_save_memory(
    State(_state): State<DbState>,
    Json(req): Json<SaveMemoryRequest>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    let save_req = service::SaveMemoryRequest {
        text: req.text,
        title: req.title,
        project: req.project,
        session_id: req.session_id,
        host: req.host.or_else(|| Some("api".to_string())),
        topic_key: req.topic_key,
        memory_type: req.memory_type,
        files: req.files,
        scope: req.scope,
        created_at_epoch: req.created_at_epoch,
        branch: req.branch,
        local_path: req.local_path,
        local_copy_enabled: req.local_copy_enabled,
        claim_enabled: req.claim_enabled,
        claim_source: req.claim_source.or_else(|| Some("api_save".to_string())),
    };

    match service::save_memory(&conn, &save_req) {
        Ok(saved) => (
            StatusCode::CREATED,
            Json(SaveMemoryResponse {
                id: saved.id,
                status: saved.status,
                memory_type: saved.memory_type,
                project: saved.project,
                scope: saved.scope,
                topic_key: saved.topic_key,
                branch: saved.branch,
                operation: saved.operation,
                created_at_epoch: saved.created_at_epoch,
                updated_at_epoch: saved.updated_at_epoch,
                upserted: saved.upserted,
                local_copy: LocalCopyResponse {
                    status: saved.local_copy.status,
                    path: saved.local_copy.path,
                    reason: saved.local_copy.reason,
                },
                local_status: saved.local_status,
                local_path: saved.local_path,
                claim_status: saved.claim_status,
                claim_id: saved.claim_id,
                claim_error: saved.claim_error,
                next_step: SaveMemoryNextStepResponse {
                    tool: saved.next_step.tool,
                    ids: saved.next_step.ids,
                    source: saved.next_step.source,
                    reason: saved.next_step.reason,
                },
            }),
        )
            .into_response(),
        Err(err) => {
            let msg = err.to_string();
            let status = if msg.contains("outside the allowed directory") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            error_response(status, "save_failed", &msg).into_response()
        }
    }
}
