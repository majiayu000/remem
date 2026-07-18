use axum::{
    middleware,
    routing::{get, post},
    Extension, Router,
};

use super::auth::{ensure_api_token, require_api_token};
use super::handlers::{
    handle_approve_candidate, handle_archive_memory, handle_blocked_candidates,
    handle_candidate_detail, handle_capabilities, handle_edit_candidate, handle_event_detail,
    handle_get_memory, handle_graph, handle_health, handle_list_candidates, handle_list_events,
    handle_list_memories, handle_list_observations, handle_list_sessions, handle_list_tasks,
    handle_list_workstreams, handle_memory_detail, handle_observation_detail,
    handle_reject_candidate, handle_restore_memory, handle_safe_approve_candidate,
    handle_safe_edit_candidate, handle_safe_reject_candidate, handle_save_memory, handle_search,
    handle_session_detail, handle_stats, handle_status, handle_task_detail, handle_user_recall,
    handle_workstream_detail,
};
use super::types::{DbState, StatusCache};

pub fn build_router(_port: u16) -> Router<DbState> {
    Router::new()
        .route("/api/v1/health", get(handle_health))
        .route("/api/v1/capabilities", get(handle_capabilities))
        .route("/api/v1/search", get(handle_search))
        .route("/api/v1/memory", get(handle_get_memory))
        .route(
            "/api/v1/memories",
            get(handle_list_memories).post(handle_save_memory),
        )
        .route("/api/v1/user/recall", post(handle_user_recall))
        .route("/api/v1/status", get(handle_status))
        .route("/api/v1/memories/list", get(handle_list_memories))
        .route("/api/v1/memories/{id}", get(handle_memory_detail))
        .route("/api/v1/memories/{id}/archive", post(handle_archive_memory))
        .route("/api/v1/memories/{id}/restore", post(handle_restore_memory))
        .route("/api/v1/candidates", get(handle_list_candidates))
        .route("/api/v1/candidates/blocked", get(handle_blocked_candidates))
        .route("/api/v1/candidates/{id}", get(handle_candidate_detail))
        .route(
            "/api/v1/candidates/{id}/approve",
            post(handle_approve_candidate),
        )
        .route(
            "/api/v1/candidates/{id}/reject",
            post(handle_reject_candidate),
        )
        .route("/api/v1/candidates/{id}/edit", post(handle_edit_candidate))
        .route(
            "/api/v1/candidates/{id}/review/approve",
            post(handle_safe_approve_candidate),
        )
        .route(
            "/api/v1/candidates/{id}/review/reject",
            post(handle_safe_reject_candidate),
        )
        .route(
            "/api/v1/candidates/{id}/review/edit",
            post(handle_safe_edit_candidate),
        )
        .route("/api/v1/observations", get(handle_list_observations))
        .route("/api/v1/observations/{id}", get(handle_observation_detail))
        .route("/api/v1/sessions", get(handle_list_sessions))
        .route("/api/v1/sessions/{id}", get(handle_session_detail))
        .route("/api/v1/workstreams", get(handle_list_workstreams))
        .route("/api/v1/workstreams/{id}", get(handle_workstream_detail))
        .route("/api/v1/events", get(handle_list_events))
        .route("/api/v1/events/{id}", get(handle_event_detail))
        .route("/api/v1/tasks", get(handle_list_tasks))
        .route("/api/v1/tasks/{id}", get(handle_task_detail))
        .route("/api/v1/graph", get(handle_graph))
        .route("/api/v1/stats", get(handle_stats))
        .route_layer(middleware::from_fn(require_api_token))
        .layer(Extension(StatusCache::default()))
}

pub async fn run_api_server(port: u16) -> anyhow::Result<()> {
    let token_path = ensure_api_token()?;
    let app = build_router(port).with_state(DbState);
    let addr = format!("127.0.0.1:{}", port);

    crate::log::info("api", &format!("REST API listening on http://{}", addr));
    println!(
        "remem REST API v{} on http://{}",
        env!("CARGO_PKG_VERSION"),
        addr
    );
    println!(
        "API token: Authorization: Bearer $(cat {})",
        token_path.display()
    );

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
