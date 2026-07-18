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
    endpoints.extend(read_resource_endpoint_bundle(
        ReadResourceFeatureSet::default(),
    ));
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
            observations: false,
            sessions: false,
            workstreams: false,
            events: false,
            tasks: false,
            graph: true,
            user_recall: true,
            user_recall_usage_policy: true,
        },
        endpoints,
    })
}

#[derive(Clone, Copy, Default)]
struct ReadResourceFeatureSet {
    observations: bool,
    sessions: bool,
    workstreams: bool,
    events: bool,
    tasks: bool,
}

fn read_resource_endpoint_bundle(
    features: ReadResourceFeatureSet,
) -> BTreeMap<&'static str, &'static str> {
    let mut endpoints = BTreeMap::new();
    insert_resource_pair(
        &mut endpoints,
        features.observations,
        ("observations_list", "/api/v1/observations"),
        ("observations_detail", "/api/v1/observations/{id}"),
    );
    insert_resource_pair(
        &mut endpoints,
        features.sessions,
        ("sessions_list", "/api/v1/sessions"),
        ("sessions_detail", "/api/v1/sessions/{id}"),
    );
    insert_resource_pair(
        &mut endpoints,
        features.workstreams,
        ("workstreams_list", "/api/v1/workstreams"),
        ("workstreams_detail", "/api/v1/workstreams/{id}"),
    );
    insert_resource_pair(
        &mut endpoints,
        features.events,
        ("events_list", "/api/v1/events"),
        ("events_detail", "/api/v1/events/{id}"),
    );
    insert_resource_pair(
        &mut endpoints,
        features.tasks,
        ("tasks_list", "/api/v1/tasks"),
        ("tasks_detail", "/api/v1/tasks/{id}"),
    );
    endpoints
}

fn insert_resource_pair(
    endpoints: &mut BTreeMap<&'static str, &'static str>,
    enabled: bool,
    list: (&'static str, &'static str),
    detail: (&'static str, &'static str),
) {
    if enabled {
        endpoints.insert(list.0, list.1);
        endpoints.insert(detail.0, detail.1);
    }
}

#[cfg(test)]
fn resource_pair_is_atomic(
    enabled: bool,
    endpoints: &BTreeMap<&'static str, &'static str>,
    list_key: &str,
    detail_key: &str,
) -> bool {
    endpoints.contains_key(list_key) == enabled && endpoints.contains_key(detail_key) == enabled
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

    #[test]
    fn staged_read_resource_endpoint_bundles_have_exact_atomic_pairs() {
        assert!(read_resource_endpoint_bundle(ReadResourceFeatureSet::default()).is_empty());
        let all_enabled = ReadResourceFeatureSet {
            observations: true,
            sessions: true,
            workstreams: true,
            events: true,
            tasks: true,
        };
        let endpoints = read_resource_endpoint_bundle(all_enabled);
        assert_eq!(
            endpoints,
            BTreeMap::from([
                ("observations_list", "/api/v1/observations"),
                ("observations_detail", "/api/v1/observations/{id}"),
                ("sessions_list", "/api/v1/sessions"),
                ("sessions_detail", "/api/v1/sessions/{id}"),
                ("workstreams_list", "/api/v1/workstreams"),
                ("workstreams_detail", "/api/v1/workstreams/{id}"),
                ("events_list", "/api/v1/events"),
                ("events_detail", "/api/v1/events/{id}"),
                ("tasks_list", "/api/v1/tasks"),
                ("tasks_detail", "/api/v1/tasks/{id}"),
            ])
        );
        for (list, detail) in [
            ("observations_list", "observations_detail"),
            ("sessions_list", "sessions_detail"),
            ("workstreams_list", "workstreams_detail"),
            ("events_list", "events_detail"),
            ("tasks_list", "tasks_detail"),
        ] {
            assert!(resource_pair_is_atomic(true, &endpoints, list, detail));
            let mut partial = endpoints.clone();
            partial.remove(detail);
            assert!(!resource_pair_is_atomic(true, &partial, list, detail));
        }

        for (features, list, detail) in [
            (
                ReadResourceFeatureSet {
                    observations: true,
                    ..Default::default()
                },
                "observations_list",
                "observations_detail",
            ),
            (
                ReadResourceFeatureSet {
                    sessions: true,
                    ..Default::default()
                },
                "sessions_list",
                "sessions_detail",
            ),
            (
                ReadResourceFeatureSet {
                    workstreams: true,
                    ..Default::default()
                },
                "workstreams_list",
                "workstreams_detail",
            ),
            (
                ReadResourceFeatureSet {
                    events: true,
                    ..Default::default()
                },
                "events_list",
                "events_detail",
            ),
            (
                ReadResourceFeatureSet {
                    tasks: true,
                    ..Default::default()
                },
                "tasks_list",
                "tasks_detail",
            ),
        ] {
            let pair = read_resource_endpoint_bundle(features);
            assert_eq!(pair.len(), 2);
            assert!(resource_pair_is_atomic(true, &pair, list, detail));
        }
    }
}
