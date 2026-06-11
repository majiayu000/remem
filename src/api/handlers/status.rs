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
    let capture_spill = match crate::observe::capture_spill_stats() {
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
        "captured_events": stats.captured_events,
        "capture_audit_events": stats.capture_audit_events,
        "capture_audit_reasons": stats.capture_audit_reasons
            .into_iter()
            .map(|reason| serde_json::json!({
                "reason": reason.reason,
                "total": reason.total,
            }))
            .collect::<Vec<_>>(),
        "pending_capture_spill_files": capture_spill.pending_files,
        "pending_capture_spill_bytes": capture_spill.pending_bytes,
        "pending_extraction_tasks": stats.pending_extraction_tasks,
        "pending_memory_candidates": stats.pending_memory_candidates,
        "pending_graph_candidates": stats.pending_graph_candidates,
    }))
    .into_response()
}
