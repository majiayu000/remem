use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::memory;

use super::super::helpers::{error_response, memory_to_item, open_request_db};
use super::super::types::{DbState, ShowParams};

pub(in crate::api) async fn handle_get_memory(
    State(_state): State<DbState>,
    Query(params): Query<ShowParams>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    match memory::get_memories_by_ids(&conn, &[params.id], None) {
        Ok(results) if !results.is_empty() => Json(memory_to_item(&results[0])).into_response(),
        Ok(_) => {
            error_response(StatusCode::NOT_FOUND, "not_found", "Memory not found").into_response()
        }
        Err(err) => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "db_error", &err.to_string())
                .into_response()
        }
    }
}
