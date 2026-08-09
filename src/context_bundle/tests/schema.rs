//! Schema snapshot tests: the serialized JSON structure of the v1 plan
//! and bundle is pinned. Any change here is a contract change and must
//! bump the schema or policy version deliberately.

use serde_json::json;

use super::{item, request, session_start_plan};
use crate::context_bundle::{execute, ChannelKind, ContextIntent, ExecutorInputs};

/// Stable across processes and runs: same request + compiled policy.
const EXPECTED_PLAN_HASH: &str = "997a834168f6f85d53f091156fe500344e3f6df157bd1177e2fc1cb8655f2697";

/// The plan's top-level key set is the contract boundary. Pinning it
/// catches an added/removed/renamed field, which must be a deliberate
/// schema or policy version bump. Per-channel values are locked by
/// `retrieval_router::tests`; repeating all 15 channel plans here would
/// duplicate that coverage without adding contract signal.
#[test]
fn plan_json_top_level_keys_are_pinned() {
    let planned = session_start_plan(&request());
    let actual = serde_json::to_value(&planned).expect("json");
    let mut keys: Vec<&str> = actual
        .as_object()
        .expect("plan is a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();

    assert_eq!(
        keys,
        [
            "abstention_policy",
            "channel_plans",
            "filters",
            "freshness_policy",
            "intent",
            "intent_source",
            "output_sections",
            "plan_hash",
            "policy_version",
            "reason_codes",
            "relevance_k",
            "relevance_policy_version",
            "relevance_query",
            "rerank_policy",
            "risk",
            "role",
            "schema_version",
            "section_budgets",
            "token_budget",
            "trust_policy",
        ]
    );
}

/// The output-section side of the plan: what the executor fills and how
/// much budget each section gets.
#[test]
fn plan_output_section_schema_snapshot() {
    let planned = session_start_plan(&request());
    let actual = serde_json::to_value(&planned).expect("json");

    assert_eq!(actual["schema_version"], json!(1));
    assert_eq!(actual["policy_version"], json!("retrieval_router_v2"));
    assert_eq!(
        actual["relevance_policy_version"],
        json!("sessionstart_significant_token_v1")
    );
    assert_eq!(actual["intent"], json!("session_start"));
    assert_eq!(actual["intent_source"], json!("explicit"));
    assert_eq!(
        actual["relevance_query"],
        json!("fix startup migration races")
    );
    assert_eq!(actual["relevance_k"], json!(1));
    assert_eq!(actual["plan_hash"], json!(EXPECTED_PLAN_HASH));
    assert_eq!(
        actual["output_sections"],
        json!([
            {"channel": "preferences", "item_limit": 25, "relevance_governed": false},
            {"channel": "lessons", "item_limit": 4, "relevance_governed": true},
            {"channel": "core", "item_limit": 6, "relevance_governed": false},
            {"channel": "workstreams", "item_limit": 5, "relevance_governed": false},
            {"channel": "memory_index", "item_limit": 50, "relevance_governed": true},
            {"channel": "sessions", "item_limit": 5, "relevance_governed": true},
        ])
    );
    assert_eq!(
        actual["filters"],
        json!({
            "project": "demo/project",
            "branch": "main",
            "include_superseded": false,
            "as_of_epoch": 1_710_000_000,
        })
    );
    assert_eq!(
        actual["section_budgets"],
        json!({
            "total_tokens": 3000,
            "preferences": 375,
            "lessons": 300,
            "core": 750,
            "workstreams": 300,
            "memory_index": 1000,
            "sessions": 550,
        })
    );
}

/// Task intents produce a ranked result list, not sections; only
/// SessionStart plans sections.
#[test]
fn task_intents_plan_no_output_sections() {
    let planned =
        crate::retrieval_router::plan(&request(), Some(ContextIntent::DebugFailure)).expect("plan");
    assert!(planned.output_sections.is_empty());
}

#[test]
fn bundle_json_schema_snapshot() {
    let planned = session_start_plan(&request());
    let bundle = execute(
        &planned,
        &ExecutorInputs {
            candidates: vec![item(
                "memory:9",
                ChannelKind::Core,
                "Current truth",
                "core decision",
            )],
            poisoning_drops: Vec::new(),
            preselection_drops: Vec::new(),
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
            "policy_version": "retrieval_router_v2",
            "relevance_policy_version": "sessionstart_significant_token_v1",
            "plan_hash": EXPECTED_PLAN_HASH,
            "degraded_mode": "full",
            "candidates_considered": 1,
            "selected_count": 1,
            "dropped_count": 0,
            "token_estimate": 7,
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
                    "token_estimate": 7,
                }
            ],
        },
    });
    assert_eq!(actual, expected);
}
