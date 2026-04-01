use axum::{
    routing::{get, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};

use super::handlers::{handle_get_memory, handle_save_memory, handle_search, handle_status};
use super::types::DbState;

pub fn build_router() -> Router<DbState> {
    Router::new()
        .route("/api/v1/search", get(handle_search))
        .route("/api/v1/memory", get(handle_get_memory))
        .route("/api/v1/memories", post(handle_save_memory))
        .route("/api/v1/status", get(handle_status))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
}

pub async fn run_api_server(port: u16) -> anyhow::Result<()> {
    let app = build_router().with_state(DbState);
    let addr = format!("127.0.0.1:{}", port);

    crate::log::info("api", &format!("REST API listening on http://{}", addr));
    println!(
        "remem REST API v{} on http://{}",
        env!("CARGO_PKG_VERSION"),
        addr
    );

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
