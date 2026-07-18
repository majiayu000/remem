use std::collections::BTreeMap;

use axum::{response::IntoResponse, Json};

use crate::api::types::{CapabilitiesFeatures, CapabilitiesResponse};

pub(in crate::api) async fn handle_capabilities() -> impl IntoResponse {
    let mut endpoints = BTreeMap::from([
        ("health", "/api/v1/health"),
        ("status", "/api/v1/status"),
        ("stats", "/api/v1/stats"),
        ("search", "/api/v1/search"),
        ("search_explain", "/api/v1/search?explain=true"),
        ("memory_list", "/api/v1/memories"),
        ("memory_detail", "/api/v1/memories/{id}"),
        ("save_memory", "/api/v1/memories"),
        ("candidate_rows", "/api/v1/candidates"),
        ("candidate_blocked", "/api/v1/candidates/blocked"),
        ("candidate_review", "/api/v1/candidates/{id}/approve"),
        ("graph", "/api/v1/graph"),
        ("user_recall", "/api/v1/user/recall"),
    ]);
    endpoints.extend(candidate_console_endpoint_bundle(false));
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
            candidate_filters: true,
            candidate_review: true,
            candidate_detail: false,
            candidate_evidence: false,
            candidate_review_safe: false,
            graph: true,
            user_recall: true,
            user_recall_usage_policy: true,
        },
        endpoints,
    })
}

fn candidate_console_endpoint_bundle(enabled: bool) -> BTreeMap<&'static str, &'static str> {
    if !enabled {
        return BTreeMap::new();
    }
    BTreeMap::from([
        ("candidate_detail", "/api/v1/candidates/{id}"),
        ("candidate_evidence", "/api/v1/candidates/{id}"),
        (
            "candidate_review_safe_approve",
            "/api/v1/candidates/{id}/review/approve",
        ),
        (
            "candidate_review_safe_reject",
            "/api/v1/candidates/{id}/review/reject",
        ),
        (
            "candidate_review_safe_edit",
            "/api/v1/candidates/{id}/review/edit",
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_candidate_endpoint_bundle_is_atomic() {
        assert!(candidate_console_endpoint_bundle(false).is_empty());
        assert_eq!(
            candidate_console_endpoint_bundle(true),
            BTreeMap::from([
                ("candidate_detail", "/api/v1/candidates/{id}"),
                ("candidate_evidence", "/api/v1/candidates/{id}"),
                (
                    "candidate_review_safe_approve",
                    "/api/v1/candidates/{id}/review/approve",
                ),
                (
                    "candidate_review_safe_reject",
                    "/api/v1/candidates/{id}/review/reject",
                ),
                (
                    "candidate_review_safe_edit",
                    "/api/v1/candidates/{id}/review/edit",
                ),
            ])
        );
    }
}
