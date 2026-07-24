use super::request;
use crate::context_bundle::{plan, ProjectRef, CONTEXT_BUNDLE_POLICY_VERSION};

#[test]
fn plan_is_deterministic_for_identical_requests() {
    let first = plan(&request()).expect("plan");
    let second = plan(&request()).expect("plan");

    assert_eq!(first, second);
    assert_eq!(first.plan_hash, second.plan_hash);
    assert_eq!(
        serde_json::to_string(&first).expect("json"),
        serde_json::to_string(&second).expect("json"),
    );
    assert_eq!(first.plan_hash.len(), 64);
    assert_eq!(first.policy_version, CONTEXT_BUNDLE_POLICY_VERSION);
}

#[test]
fn plan_hash_changes_when_the_request_changes() {
    let baseline = plan(&request()).expect("plan");

    let mut budget_changed = request();
    budget_changed.token_budget = 1_000;
    let budget_plan = plan(&budget_changed).expect("plan");
    assert_ne!(baseline.plan_hash, budget_plan.plan_hash);

    let mut branch_changed = request();
    branch_changed.branch = Some("feature/x".to_string());
    let branch_plan = plan(&branch_changed).expect("plan");
    assert_ne!(baseline.plan_hash, branch_plan.plan_hash);
}

#[test]
fn plan_hash_ignores_no_inputs_outside_the_request() {
    // Same request planned at "different times" must hash identically:
    // nothing time- or environment-derived may enter the hash.
    let hashes: Vec<String> = (0..3)
        .map(|_| plan(&request()).expect("plan").plan_hash)
        .collect();
    assert!(hashes.iter().all(|hash| hash == &hashes[0]));
}

#[test]
fn planner_rejects_invalid_scope_or_budget() {
    let mut empty_project = request();
    empty_project.project = ProjectRef {
        key: "  ".to_string(),
    };
    assert!(plan(&empty_project).is_err());

    let mut zero_budget = request();
    zero_budget.token_budget = 0;
    assert!(plan(&zero_budget).is_err());

    let mut wrong_schema = request();
    wrong_schema.schema_version = 999;
    assert!(plan(&wrong_schema).is_err());
}

#[test]
fn blank_task_disables_the_relevance_query() {
    let mut blank = request();
    blank.task = "   ".to_string();
    let planned = plan(&blank).expect("plan");
    assert_eq!(planned.relevance_query, None);
}
