use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use crate::db;

use super::super::helpers::{error_response, open_request_db};
use super::super::types::DbState;

pub(in crate::api) async fn handle_status(State(_state): State<DbState>) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    let stats = match db::query_system_stats(&conn) {
        Ok(stats) => stats,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "status_failed",
                &err.to_string(),
            )
            .into_response();
        }
    };

    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "memories": stats.active_memories,
        "observations": stats.active_observations,
    }))
    .into_response()
}
