use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::{db, memory};

use super::types::{ErrorDetail, ErrorResponse, MemoryItem};

pub(super) fn memory_to_item_with_conn(
    conn: &rusqlite::Connection,
    memory: &memory::Memory,
) -> MemoryItem {
    let now_epoch = chrono::Utc::now().timestamp();
    let staleness = match memory::memory_staleness_label_with_conn(conn, memory, now_epoch) {
        Ok(label) => label,
        Err(err) => {
            crate::log::warn(
                "api",
                &format!(
                    "memory {} staleness source-anchor lookup failed: {}",
                    memory.id, err
                ),
            );
            memory::memory_staleness_label(memory, now_epoch)
        }
    };
    memory_to_item_with_staleness(memory, staleness)
}

fn memory_to_item_with_staleness(
    memory: &memory::Memory,
    staleness: memory::MemoryStalenessLabel,
) -> MemoryItem {
    MemoryItem {
        id: memory.id,
        title: memory.title.clone(),
        content: memory.text.clone(),
        memory_type: memory.memory_type.clone(),
        project: memory.project.clone(),
        scope: memory.scope.clone(),
        status: memory.status.clone(),
        staleness,
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

#[allow(clippy::result_large_err)]
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
