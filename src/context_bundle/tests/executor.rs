use super::{item, request, session_start_plan};
use crate::context_bundle::{
    execute, ChannelKind, DegradedMode, ExecutorInputs, ItemValidity, SourceKind, TrustClass,
};

fn inputs(candidates: Vec<crate::context_bundle::ContextItem>) -> ExecutorInputs {
    ExecutorInputs {
        candidates,
        enrichment_available: true,
    }
}

fn reason_for(bundle: &crate::context_bundle::ContextBundle, stable_key: &str) -> String {
    bundle
        .audit
        .entries
        .iter()
        .find(|entry| entry.stable_key == stable_key)
        .unwrap_or_else(|| panic!("missing audit entry for {stable_key}"))
        .reason
        .clone()
}

#[test]
fn executor_reuses_sessionstart_relevance_selection() {
    let planned = session_start_plan(&request());
    let candidates = vec![
        item(
            "memory:1",
            ChannelKind::Lessons,
            "Startup migration races",
            "startup migration locking fix",
        ),
        item(
            "memory:2",
            ChannelKind::MemoryIndex,
            "Unrelated note",
            "completely different topic",
        ),
        item(
            "memory:9",
            ChannelKind::Core,
            "Current truth",
            "core decision",
        ),
    ];
    let bundle = execute(&planned, &inputs(candidates));

    assert_eq!(bundle.degraded_mode, DegradedMode::Full);
    // relevance_k defaults to 1: the matching lesson wins the slot, the
    // unrelated index item drops with the SessionStart reason string.
    assert_eq!(bundle.failure_lessons.len(), 1);
    assert_eq!(bundle.failure_lessons[0].stable_key, "memory:1");
    assert!(bundle.memory_index.is_empty());
    assert_eq!(
        reason_for(&bundle, "memory:2"),
        "below_sessionstart_relevance_threshold"
    );
    // Core is not relevance-governed.
    assert_eq!(bundle.current_truth.len(), 1);
    assert_eq!(reason_for(&bundle, "memory:9"), "channel_default_selected");
    assert_eq!(bundle.audit.candidates_considered, 3);
    assert_eq!(bundle.audit.selected_count, 2);
    assert_eq!(bundle.audit.dropped_count, 1);
}

#[test]
fn executor_drops_out_of_scope_quarantined_and_superseded_items() {
    let planned = session_start_plan(&request());
    let mut other_project = item("memory:10", ChannelKind::Core, "Other", "text");
    other_project.project = Some("other/project".to_string());
    let mut other_branch = item("memory:11", ChannelKind::Core, "Branch", "text");
    other_branch.branch = Some("feature/y".to_string());
    let mut superseded = item("memory:12", ChannelKind::Core, "Old", "text");
    superseded.validity = ItemValidity::Superseded;
    let mut quarantined = item("memory:13", ChannelKind::Core, "Poison", "text");
    quarantined.trust = TrustClass::Quarantined;

    let bundle = execute(
        &planned,
        &inputs(vec![other_project, other_branch, superseded, quarantined]),
    );

    assert!(bundle.current_truth.is_empty());
    assert_eq!(reason_for(&bundle, "memory:10"), "project_scope_mismatch");
    assert_eq!(reason_for(&bundle, "memory:11"), "branch_scope_mismatch");
    assert_eq!(reason_for(&bundle, "memory:12"), "superseded_excluded");
    assert_eq!(reason_for(&bundle, "memory:13"), "quarantined_trust");
}

#[test]
fn include_superseded_keeps_superseded_items() {
    let mut with_superseded = request();
    with_superseded.include_superseded = true;
    let planned = session_start_plan(&with_superseded);
    let mut superseded = item("memory:12", ChannelKind::Core, "Old", "text");
    superseded.validity = ItemValidity::Superseded;

    let bundle = execute(&planned, &inputs(vec![superseded]));

    assert_eq!(bundle.current_truth.len(), 1);
}

#[test]
fn missing_enrichment_degrades_to_canonical_only() {
    let planned = session_start_plan(&request());
    let mut generated = item("memory:20", ChannelKind::Core, "Derived", "generated text");
    generated.source_kind = SourceKind::Generated;
    let canonical = item(
        "memory:21",
        ChannelKind::Core,
        "Canonical",
        "canonical text",
    );
    let bundle = execute(
        &planned,
        &ExecutorInputs {
            candidates: vec![generated, canonical],
            enrichment_available: false,
        },
    );

    assert_eq!(bundle.degraded_mode, DegradedMode::CanonicalOnly);
    assert_eq!(bundle.audit.degraded_mode, DegradedMode::CanonicalOnly);
    assert_eq!(reason_for(&bundle, "memory:20"), "canonical_only_degraded");
    assert_eq!(bundle.current_truth.len(), 1);
    assert_eq!(bundle.current_truth[0].stable_key, "memory:21");
}

#[test]
fn executor_enforces_channel_item_limit_and_token_budgets() {
    let mut planned = session_start_plan(&request());
    for channel in &mut planned.output_sections {
        if channel.channel == ChannelKind::Core {
            channel.item_limit = 1;
        }
    }
    let bundle = execute(
        &planned,
        &inputs(vec![
            item("memory:30", ChannelKind::Core, "First", "text"),
            item("memory:31", ChannelKind::Core, "Second", "text"),
        ]),
    );
    assert_eq!(bundle.current_truth.len(), 1);
    assert_eq!(reason_for(&bundle, "memory:31"), "channel_item_limit");

    let mut planned = session_start_plan(&request());
    planned.section_budgets.core = 4;
    let bundle = execute(
        &planned,
        &inputs(vec![
            item("memory:32", ChannelKind::Core, "Fits", "0123456789"),
            item("memory:33", ChannelKind::Core, "Too big", &"x".repeat(200)),
        ]),
    );
    assert_eq!(bundle.current_truth.len(), 1);
    assert_eq!(reason_for(&bundle, "memory:33"), "channel_token_budget");
}

#[test]
fn executor_enforces_the_total_token_budget_and_records_truncation() {
    let mut planned = session_start_plan(&request());
    planned.section_budgets.total_tokens = 5;
    let bundle = execute(
        &planned,
        &inputs(vec![
            item("memory:40", ChannelKind::Core, "Fits", "0123456789"),
            item("memory:41", ChannelKind::Core, "Over", "0123456789012345"),
        ]),
    );

    assert_eq!(bundle.current_truth.len(), 1);
    assert_eq!(reason_for(&bundle, "memory:41"), "total_token_budget");
    assert_eq!(
        bundle.audit.truncation_reason.as_deref(),
        Some("total_token_budget")
    );
    assert!(bundle.audit.token_estimate <= planned.section_budgets.total_tokens);
}

#[test]
fn tampered_plan_produces_a_blocked_bundle() {
    let mut planned = session_start_plan(&request());
    planned.policy_version = "context_bundle_v0_forged".to_string();
    let bundle = execute(
        &planned,
        &inputs(vec![item("memory:50", ChannelKind::Core, "Any", "text")]),
    );

    assert_eq!(bundle.degraded_mode, DegradedMode::Blocked);
    assert!(bundle.current_truth.is_empty());
    assert_eq!(bundle.audit.selected_count, 0);
    assert_eq!(reason_for(&bundle, "memory:50"), "plan_blocked");
}

#[test]
fn execution_is_deterministic_for_identical_inputs() {
    let planned = session_start_plan(&request());
    let candidates = vec![
        item(
            "memory:1",
            ChannelKind::Lessons,
            "Startup migration races",
            "startup migration locking fix",
        ),
        item(
            "memory:9",
            ChannelKind::Core,
            "Current truth",
            "core decision",
        ),
    ];
    let first = execute(&planned, &inputs(candidates.clone()));
    let second = execute(&planned, &inputs(candidates));

    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).expect("json"),
        serde_json::to_string(&second).expect("json"),
    );
}
