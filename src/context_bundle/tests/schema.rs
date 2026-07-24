//! Schema snapshot tests: the serialized JSON structure of the v1 plan
//! and bundle is pinned. Any change here is a contract change and must
//! bump the schema or policy version deliberately.

use serde_json::json;

use super::{item, request};
use crate::context_bundle::{execute, plan, ChannelKind, ExecutorInputs};

/// Stable across processes and runs: same request + compiled policy.
const EXPECTED_PLAN_HASH: &str = "2068028822d3c7f14a5e20bad6265e5a910034861ad63eccc82cddba02376ad6";

#[test]
fn plan_json_schema_snapshot() {
    let planned = plan(&request()).expect("plan");
    let actual = serde_json::to_value(&planned).expect("json");

    let expected = json!({
        "schema_version": 1,
        "policy_version": "context_bundle_v1",
        "relevance_policy_version": "sessionstart_significant_token_v1",
        "intent": "session_start",
        "relevance_query": "fix startup migration races",
        "relevance_k": 1,
        "channels": [
            {"channel": "preferences", "item_limit": 25, "relevance_governed": false},
            {"channel": "lessons", "item_limit": 4, "relevance_governed": true},
            {"channel": "core", "item_limit": 6, "relevance_governed": false},
            {"channel": "workstreams", "item_limit": 5, "relevance_governed": false},
            {"channel": "memory_index", "item_limit": 50, "relevance_governed": true},
            {"channel": "sessions", "item_limit": 5, "relevance_governed": true},
        ],
        "filters": {
            "project": "demo/project",
            "branch": "main",
            "include_superseded": false,
            "as_of_epoch": 1_710_000_000,
        },
        "section_budgets": {
            "total_tokens": 3000,
            "preferences": 375,
            "lessons": 300,
            "core": 750,
            "workstreams": 300,
            "memory_index": 1000,
            "sessions": 550,
        },
        "plan_hash": EXPECTED_PLAN_HASH,
    });
    assert_eq!(actual, expected);
}

#[test]
fn bundle_json_schema_snapshot() {
    let planned = plan(&request()).expect("plan");
    let bundle = execute(
        &planned,
        &ExecutorInputs {
            candidates: vec![item(
                "memory:9",
                ChannelKind::Core,
                "Current truth",
                "core decision",
            )],
            enrichment_available: true,
        },
    );
    let actual = serde_json::to_value(&bundle).expect("json");

    let expected = json!({
        "schema_version": 1,
        "plan_hash": EXPECTED_PLAN_HASH,
        "degraded_mode": "full",
        "preferences": [],
        "failure_lessons": [],
        "current_truth": [
            {
                "stable_key": "memory:9",
                "channel": "core",
                "title": "Current truth",
                "text": "core decision",
                "source_kind": "canonical",
                "canonical_ref": "memory:9@canonical",
                "projection_ref": null,
                "evidence_refs": [],
                "validity": "current",
                "trust": "standard",
                "project": "demo/project",
                "branch": null,
            }
        ],
        "workstreams": [],
        "memory_index": [],
        "recent_sessions": [],
        "audit": {
            "schema_version": 1,
            "policy_version": "context_bundle_v1",
            "relevance_policy_version": "sessionstart_significant_token_v1",
            "plan_hash": EXPECTED_PLAN_HASH,
            "degraded_mode": "full",
            "candidates_considered": 1,
            "selected_count": 1,
            "dropped_count": 0,
            "token_estimate": 4,
            "token_budget": 3000,
            "truncation_reason": null,
            "entries": [
                {
                    "stable_key": "memory:9",
                    "channel": "core",
                    "source_kind": "canonical",
                    "validity": "current",
                    "selected": true,
                    "reason": "channel_default_selected",
                    "relevance_score": null,
                    "token_estimate": 4,
                }
            ],
        },
    });
    assert_eq!(actual, expected);
}
