use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use super::super::helpers::{error_response, open_request_db};
use super::super::types::{DbState, StatsResponse, TypeCount};

pub(in crate::api) async fn handle_stats(State(_state): State<DbState>) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    let result = (|| -> anyhow::Result<StatsResponse> {
        let current_filter =
            crate::memory::memory_current_filter_sql("status", "expires_at_epoch", false);
        let total_memories: i64 =
            conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
        let active_memories: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM memories WHERE {current_filter}"),
            [],
            |row| row.get(0),
        )?;
        let pending_candidates: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_candidates WHERE review_status = 'pending_review'",
            [],
            |row| row.get(0),
        )?;
        let captured_events: i64 =
            conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
        let pending_extraction_tasks: i64 = conn.query_row(
            "SELECT COUNT(*) FROM extraction_tasks WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )?;
        let ai_calls: i64 =
            conn.query_row("SELECT COUNT(*) FROM ai_usage_events", [], |row| row.get(0))?;
        let ai_cost_usd: f64 = conn.query_row(
            "SELECT COALESCE(SUM(estimated_cost_usd), 0) FROM ai_usage_events",
            [],
            |row| row.get(0),
        )?;
        let ai_total_tokens: i64 = conn.query_row(
            "SELECT COALESCE(SUM(total_tokens), 0) FROM ai_usage_events",
            [],
            |row| row.get(0),
        )?;

        let mut stmt = conn.prepare(&format!(
            "SELECT memory_type, COUNT(*) FROM memories WHERE {current_filter} \
             GROUP BY memory_type ORDER BY COUNT(*) DESC"
        ))?;
        let type_distribution: Vec<TypeCount> = stmt
            .query_map([], |row| {
                Ok(TypeCount {
                    memory_type: row.get(0)?,
                    count: row.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(StatsResponse {
            active_memories,
            total_memories,
            pending_candidates,
            captured_events,
            pending_extraction_tasks,
            ai_calls,
            ai_cost_usd,
            ai_total_tokens,
            type_distribution,
        })
    })();

    match result {
        Ok(stats) => Json(stats).into_response(),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "stats_failed",
            &err.to_string(),
        )
        .into_response(),
    }
}
