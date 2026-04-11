use axum::{
    http::{header, Method},
    routing::{get, post},
    Router,
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use super::handlers::{handle_get_memory, handle_save_memory, handle_search, handle_status};
use super::types::DbState;

pub fn build_router(port: u16) -> Router<DbState> {
    let origins: Vec<axum::http::HeaderValue> = [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ]
    .iter()
    .filter_map(|s| s.parse().ok())
    .collect();

    Router::new()
        .route("/api/v1/search", get(handle_search))
        .route("/api/v1/memory", get(handle_get_memory))
        .route("/api/v1/memories", post(handle_save_memory))
        .route("/api/v1/status", get(handle_status))
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([header::CONTENT_TYPE]),
        )
}

pub async fn run_api_server(port: u16) -> anyhow::Result<()> {
    let app = build_router(port).with_state(DbState);
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
