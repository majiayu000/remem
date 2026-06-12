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
        "total_observations": stats.total_observations,
        "captured_events": stats.captured_events,
        "capture_drop_events": stats.capture_drop_events,
        "unrecovered_capture_spills": stats.unrecovered_capture_spills,
        "pending_extraction_tasks": stats.pending_extraction_tasks,
        "pending_memory_candidates": stats.pending_memory_candidates,
        "pending_graph_candidates": stats.pending_graph_candidates,
        "promotion_funnel": {
            "captured_events": stats.captured_events,
            "observations": stats.total_observations,
            "observation_rate_percent": percent(stats.total_observations, stats.captured_events),
            "candidates": stats.total_memory_candidates,
            "candidate_rate_percent": percent(stats.total_memory_candidates, stats.total_observations),
            "promoted": stats.promoted_memory_candidates,
            "promoted_rate_percent": percent(stats.promoted_memory_candidates, stats.total_memory_candidates),
            "pending_review": stats.pending_review_memory_candidates,
            "pending_review_rate_percent": percent(stats.pending_review_memory_candidates, stats.total_memory_candidates),
        },
    }))
    .into_response()
}

fn percent(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        0.0
    } else {
        (numerator as f64 * 100.0) / denominator as f64
    }
}
