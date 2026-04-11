use axum::{
    http::{header, Method},
    routing::{get, post},
    Router,
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use super::handlers::{handle_get_memory, handle_save_memory, handle_search, handle_status};
use super::types::DbState;

pub fn build_router(_port: u16) -> Router<DbState> {
    // Allow any localhost origin (any port) so browser clients served from a
    // different port than the API (common in dev and production setups) are not
    // blocked.  Requests from non-localhost origins are still rejected.
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin: &axum::http::HeaderValue, _| {
            let b = origin.as_bytes();
            b.starts_with(b"http://localhost:")
                || b.starts_with(b"http://127.0.0.1:")
        }))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE]);

    Router::new()
        .route("/api/v1/search", get(handle_search))
        .route("/api/v1/memory", get(handle_get_memory))
        .route("/api/v1/memories", post(handle_save_memory))
        .route("/api/v1/status", get(handle_status))
        .layer(cors)
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
