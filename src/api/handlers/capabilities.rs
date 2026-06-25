use std::collections::BTreeMap;

use axum::{response::IntoResponse, Json};

use crate::api::types::{CapabilitiesFeatures, CapabilitiesResponse};

pub(in crate::api) async fn handle_capabilities() -> impl IntoResponse {
    Json(CapabilitiesResponse {
        version: crate::build_info::package_version(),
        schema_version: crate::build_info::binary_schema_version(),
        api_version: 1,
        features: CapabilitiesFeatures {
            health: true,
            status: true,
            stats: true,
            search: true,
            search_explain: true,
            memory_list: true,
            memory_detail: true,
            save_memory: true,
            candidate_rows: true,
            candidate_review: true,
            graph: true,
            user_recall: true,
            user_recall_usage_policy: true,
        },
        endpoints: BTreeMap::from([
            ("health", "/api/v1/health"),
            ("status", "/api/v1/status"),
            ("stats", "/api/v1/stats"),
            ("search", "/api/v1/search"),
            ("search_explain", "/api/v1/search?explain=true"),
            ("memory_list", "/api/v1/memories"),
            ("memory_detail", "/api/v1/memories/{id}"),
            ("save_memory", "/api/v1/memories"),
            ("candidate_rows", "/api/v1/candidates"),
            ("candidate_review", "/api/v1/candidates/{id}/approve"),
            ("graph", "/api/v1/graph"),
            ("user_recall", "/api/v1/user/recall"),
        ]),
    })
}
