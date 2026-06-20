use axum::{
    middleware,
    routing::{get, post},
    Extension, Router,
};

use super::auth::{ensure_api_token, require_api_token};
use super::handlers::{
    handle_approve_candidate, handle_capabilities, handle_edit_candidate, handle_get_memory,
    handle_graph, handle_health, handle_list_candidates, handle_list_memories,
    handle_memory_detail, handle_reject_candidate, handle_save_memory, handle_search, handle_stats,
    handle_status, handle_user_recall,
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
        .route("/api/v1/candidates", get(handle_list_candidates))
        .route(
            "/api/v1/candidates/{id}/approve",
            post(handle_approve_candidate),
        )
        .route(
            "/api/v1/candidates/{id}/reject",
            post(handle_reject_candidate),
        )
        .route("/api/v1/candidates/{id}/edit", post(handle_edit_candidate))
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
