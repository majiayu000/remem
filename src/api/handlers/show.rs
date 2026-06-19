use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::memory;

use super::super::helpers::{
    error_response, memory_to_item_with_conn, open_request_db, staleness_error_response,
};
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
        Ok(results) if !results.is_empty() => {
            let item = match memory_to_item_with_conn(&conn, &results[0]) {
                Ok(item) => item,
                Err(err) => return staleness_error_response(&err).into_response(),
            };
            if let Err(err) = memory::mark_memories_accessed(&conn, &[params.id]) {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "db_error",
                    &err.to_string(),
                )
                .into_response();
            }
            Json(item).into_response()
        }
        Ok(_) => {
            error_response(StatusCode::NOT_FOUND, "not_found", "Memory not found").into_response()
        }
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "db_error",
            &err.to_string(),
        )
        .into_response(),
    }
}
