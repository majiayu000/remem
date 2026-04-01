use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::{db, memory};

use super::types::{ErrorDetail, ErrorResponse, MemoryItem};

pub(super) fn memory_to_item(memory: &memory::Memory) -> MemoryItem {
    MemoryItem {
        id: memory.id,
        title: memory.title.clone(),
        content: memory.text.clone(),
        memory_type: memory.memory_type.clone(),
        project: memory.project.clone(),
        scope: memory.scope.clone(),
        status: memory.status.clone(),
        topic_key: memory.topic_key.clone(),
        branch: memory.branch.clone(),
        created_at_epoch: memory.created_at_epoch,
        updated_at_epoch: memory.updated_at_epoch,
    }
}

pub(super) fn error_response(status: StatusCode, code: &str, message: &str) -> impl IntoResponse {
    (
        status,
        Json(ErrorResponse {
            error: ErrorDetail {
                code: code.to_string(),
                message: message.to_string(),
            },
        }),
    )
}

pub(super) fn open_request_db() -> Result<rusqlite::Connection, Response> {
    db::open_db().map_err(|err| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "db_open_failed",
            &err.to_string(),
        )
        .into_response()
    })
}
