use axum::{response::IntoResponse, Json};

use super::super::types::HealthResponse;

pub(in crate::api) async fn handle_health() -> impl IntoResponse {
    Json(HealthResponse {
        ok: true,
        version: crate::build_info::package_version(),
        api_version: 1,
        schema_version: crate::build_info::binary_schema_version(),
    })
}
