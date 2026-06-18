use axum::{
    middleware,
    routing::{get, post},
    Router,
};

use super::auth::{ensure_api_token, require_api_token};
use super::handlers::{handle_get_memory, handle_graph, handle_list_candidates, handle_list_memories, handle_memory_detail, handle_save_memory, handle_search, handle_stats, handle_status};
use super::types::DbState;

pub fn build_router(_port: u16) -> Router<DbState> {
    Router::new()
        .route("/api/v1/search", get(handle_search))
        .route("/api/v1/memory", get(handle_get_memory))
        .route("/api/v1/memories", post(handle_save_memory))
        .route("/api/v1/status", get(handle_status))
        .route("/api/v1/memories/list", get(handle_list_memories))
        .route("/api/v1/memories/{id}", get(handle_memory_detail))
        .route("/api/v1/candidates", get(handle_list_candidates))
        .route("/api/v1/graph", get(handle_graph))
        .route("/api/v1/stats", get(handle_stats))
        .route_layer(middleware::from_fn(require_api_token))
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
