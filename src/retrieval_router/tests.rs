use crate::context_bundle::{
    AgentRole, ContextIntent, ContextRequest, ItemValidity, ProjectRef, RiskClass, TrustClass,
    CONTEXT_BUNDLE_SCHEMA_VERSION,
};

use super::domain::{
    AbstentionMode, ChannelDegradation, IntentSource, RetrievalChannel, RetrievalPlan,
    RETRIEVAL_PLAN_SCHEMA_VERSION,
};
use super::intent::resolve_intent;
use super::planner::{plan, RETRIEVAL_ROUTER_POLICY_VERSION};

fn request(task: &str) -> ContextRequest {
    ContextRequest {
        schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
        task: task.to_string(),
        project: ProjectRef {
            key: "demo/project".to_string(),
        },
        branch: Some("main".to_string()),
        worktree: None,
        role: AgentRole::Coder,
        as_of_epoch: 1_710_000_000,
        token_budget: 4_000,
        risk: RiskClass::Medium,
        include_superseded: false,
    }
}

fn enabled(plan: &RetrievalPlan) -> Vec<RetrievalChannel> {
    plan.enabled_channels()
}

// --- intent resolution -------------------------------------------------

#[test]
fn explicit_intent_wins_over_keywords() {
    let resolved = resolve_intent(Some(ContextIntent::ResumeWork), "why did this error happen");
    assert_eq!(resolved.intent, ContextIntent::ResumeWork);
    assert_eq!(resolved.source, IntentSource::Explicit);
    assert_eq!(resolved.reason_code, "explicit_intent");
}

#[test]
fn explicit_session_start_falls_back_to_generic_policy() {
    let resolved = resolve_intent(Some(ContextIntent::SessionStart), "anything");
    assert_eq!(resolved.intent, ContextIntent::ExploreHistory);
    assert_eq!(resolved.source, IntentSource::DefaultFallback);
    assert_eq!(resolved.reason_code, "session_start_not_routable");
}

#[test]
fn keyword_fallback_classifies_each_intent() {
    let cases = [
        (
            "debug the panic in worker startup",
            ContextIntent::DebugFailure,
        ),
        (
            "why did we pick sqlite over postgres",
            ContextIntent::ExplainDecision,
        ),
        (
            "review the pending diff before merge",
            ContextIntent::ReviewChange,
        ),
        ("resume the migration work", ContextIntent::ResumeWork),
        (
            "apply the project coding style",
            ContextIntent::ApplyPreference,
        ),
    ];
    for (task, want) in cases {
        let resolved = resolve_intent(None, task);
        assert_eq!(resolved.intent, want, "task {task:?}");
        assert_eq!(
            resolved.source,
            IntentSource::KeywordFallback,
            "task {task:?}"
        );
        assert!(
            resolved.reason_code.starts_with("keyword_match_"),
            "task {task:?} reason {}",
            resolved.reason_code
        );
    }
}

#[test]
fn keyword_priority_order_is_fixed_debug_over_decision() {
    // Contains both "why" and "error": DebugFailure rules run first.
    let resolved = resolve_intent(None, "why does this error keep happening");
    assert_eq!(resolved.intent, ContextIntent::DebugFailure);
}

#[test]
fn unclassified_task_conservatively_falls_back() {
    let resolved = resolve_intent(None, "quarterly llama farming report");
    assert_eq!(resolved.intent, ContextIntent::ExploreHistory);
    assert_eq!(resolved.source, IntentSource::DefaultFallback);
    assert_eq!(resolved.reason_code, "unclassified_conservative_fallback");
}

// --- per-intent channel mapping (locked) -------------------------------

const BASELINE: [RetrievalChannel; 3] = [
    RetrievalChannel::CanonicalFts,
    RetrievalChannel::CanonicalVector,
    RetrievalChannel::GeneratedEnrichment,
];

fn assert_enabled_set(intent: ContextIntent, expected_priority: &[RetrievalChannel]) {
    let p = plan(&request("task"), Some(intent)).unwrap();
    let mut want: Vec<RetrievalChannel> = BASELINE.to_vec();
    want.extend_from_slice(expected_priority);
    want.sort();
    let mut got = enabled(&p);
    got.sort();
    assert_eq!(got, want, "intent {intent:?}");
    // Every known channel appears exactly once, enabled or not.
    assert_eq!(p.channel_plans.len(), RetrievalChannel::ORDERED.len());
}

#[test]
fn resume_work_channel_mapping_locked() {
    assert_enabled_set(
        ContextIntent::ResumeWork,
        &[
            RetrievalChannel::Workstreams,
            RetrievalChannel::SessionOutcomes,
            RetrievalChannel::Decisions,
            RetrievalChannel::Temporal,
        ],
    );
}

#[test]
fn explain_decision_channel_mapping_locked() {
    assert_enabled_set(
        ContextIntent::ExplainDecision,
        &[
            RetrievalChannel::Decisions,
            RetrievalChannel::SupersededHistory,
            RetrievalChannel::GitEvidence,
            RetrievalChannel::BenchmarkEvidence,
            RetrievalChannel::Temporal,
        ],
    );
}

#[test]
fn debug_failure_channel_mapping_locked() {
    assert_enabled_set(
        ContextIntent::DebugFailure,
        &[
            RetrievalChannel::FailureLessons,
            RetrievalChannel::EntityGraph,
            RetrievalChannel::GitEvidence,
            RetrievalChannel::SessionOutcomes,
        ],
    );
}

#[test]
fn apply_preference_channel_mapping_locked() {
    assert_enabled_set(
        ContextIntent::ApplyPreference,
        &[
            RetrievalChannel::Preferences,
            RetrievalChannel::Constraints,
            RetrievalChannel::SupersededHistory,
        ],
    );
}

#[test]
fn review_change_channel_mapping_locked() {
    assert_enabled_set(
        ContextIntent::ReviewChange,
        &[
            RetrievalChannel::Constraints,
            RetrievalChannel::FailureLessons,
            RetrievalChannel::Decisions,
            RetrievalChannel::GitEvidence,
        ],
    );
}

#[test]
fn explore_history_channel_mapping_locked() {
    assert_enabled_set(
        ContextIntent::ExploreHistory,
        &[
            RetrievalChannel::Temporal,
            RetrievalChannel::SessionOutcomes,
            RetrievalChannel::SupersededHistory,
            RetrievalChannel::GraphExpansion,
        ],
    );
}

#[test]
fn explain_decision_top_priority_is_decisions() {
    let p = plan(&request("t"), Some(ContextIntent::ExplainDecision)).unwrap();
    let top = p
        .channel_plans
        .iter()
        .filter(|c| c.enabled)
        .max_by(|a, b| a.weight.partial_cmp(&b.weight).unwrap())
        .unwrap();
    assert_eq!(top.channel, RetrievalChannel::Decisions);
    assert_eq!(top.weight, 1.0);
}

// --- policy compilation ------------------------------------------------

#[test]
fn plan_carries_versions_scope_and_budget() {
    let req = request("explore");
    let p = plan(&req, Some(ContextIntent::ExploreHistory)).unwrap();
    assert_eq!(p.schema_version, RETRIEVAL_PLAN_SCHEMA_VERSION);
    assert_eq!(p.policy_version, RETRIEVAL_ROUTER_POLICY_VERSION);
    assert_eq!(p.policy_version, "retrieval_router_v2");
    assert_eq!(p.filters.project, "demo/project");
    assert_eq!(p.filters.branch.as_deref(), Some("main"));
    assert_eq!(p.filters.as_of_epoch, 1_710_000_000);
    assert_eq!(p.token_budget, 4_000);
    assert_eq!(p.role, AgentRole::Coder);
    assert_eq!(p.risk, RiskClass::Medium);
    assert!(!p.plan_hash.is_empty());
}

#[test]
fn enrichment_is_capped_secondary_signal() {
    for intent in [
        ContextIntent::ResumeWork,
        ContextIntent::ExplainDecision,
        ContextIntent::DebugFailure,
        ContextIntent::ApplyPreference,
        ContextIntent::ReviewChange,
        ContextIntent::ExploreHistory,
    ] {
        let p = plan(&request("t"), Some(intent)).unwrap();
        let enrichment = p
            .channel_plans
            .iter()
            .find(|c| c.channel == RetrievalChannel::GeneratedEnrichment)
            .unwrap();
        let fts = p
            .channel_plans
            .iter()
            .find(|c| c.channel == RetrievalChannel::CanonicalFts)
            .unwrap();
        assert!(enrichment.enabled, "intent {intent:?}");
        assert!(enrichment.weight < fts.weight, "intent {intent:?}");
        assert!(enrichment.max_contribution <= 2, "intent {intent:?}");
        assert_eq!(enrichment.degradation, ChannelDegradation::SkipChannel);
    }
}

#[test]
fn rerank_enabled_only_for_evidence_heavy_intents() {
    let on = [
        ContextIntent::ExplainDecision,
        ContextIntent::DebugFailure,
        ContextIntent::ReviewChange,
    ];
    let off = [
        ContextIntent::ResumeWork,
        ContextIntent::ApplyPreference,
        ContextIntent::ExploreHistory,
    ];
    for intent in on {
        let p = plan(&request("t"), Some(intent)).unwrap();
        assert!(p.rerank_policy.enabled, "intent {intent:?}");
        assert_eq!(p.rerank_policy.candidate_pool, 50);
        assert_eq!(p.rerank_policy.output_k, 10);
    }
    for intent in off {
        let p = plan(&request("t"), Some(intent)).unwrap();
        assert!(!p.rerank_policy.enabled, "intent {intent:?}");
    }
}

#[test]
fn high_risk_tightens_trust_enrichment_rerank_and_abstention() {
    let mut req = request("review the diff");
    req.risk = RiskClass::High;
    let p = plan(&req, Some(ContextIntent::ReviewChange)).unwrap();
    assert_eq!(p.trust_policy.minimum_trust, TrustClass::Trusted);
    let enrichment = p
        .channel_plans
        .iter()
        .find(|c| c.channel == RetrievalChannel::GeneratedEnrichment)
        .unwrap();
    assert!(!enrichment.enabled);
    assert!(p.rerank_policy.require_canonical_evidence_top1);
    assert_eq!(p.abstention_policy.mode, AbstentionMode::OnLowEvidence);
    for code in [
        "high_risk_trusted_only",
        "high_risk_enrichment_disabled",
        "high_risk_canonical_evidence_top1",
        "high_risk_abstain_on_low_evidence",
    ] {
        assert!(p.reason_codes.iter().any(|r| r == code), "missing {code}");
    }
}

#[test]
fn low_risk_keeps_standard_trust_and_no_abstention() {
    let mut req = request("resume work");
    req.risk = RiskClass::Low;
    let p = plan(&req, Some(ContextIntent::ResumeWork)).unwrap();
    assert_eq!(p.trust_policy.minimum_trust, TrustClass::Standard);
    assert!(!p.trust_policy.allow_quarantined);
    assert_eq!(p.abstention_policy.mode, AbstentionMode::Never);
    assert!(!p.rerank_policy.require_canonical_evidence_top1);
}

#[test]
fn reviewer_role_enables_constraints_channel() {
    let mut req = request("debug the flaky test");
    req.role = AgentRole::Reviewer;
    let p = plan(&req, Some(ContextIntent::DebugFailure)).unwrap();
    let constraints = p
        .channel_plans
        .iter()
        .find(|c| c.channel == RetrievalChannel::Constraints)
        .unwrap();
    assert!(constraints.enabled);
    assert!(p
        .reason_codes
        .iter()
        .any(|r| r == "reviewer_constraints_enabled"));
}

fn assert_superseded_scope(plan: &RetrievalPlan, expected: bool) {
    assert_eq!(plan.filters.include_superseded, expected);
    assert_eq!(plan.freshness_policy.include_superseded, expected);
    let superseded_history = plan
        .channel_plans
        .iter()
        .find(|channel| channel.channel == RetrievalChannel::SupersededHistory)
        .unwrap();
    assert!(superseded_history.enabled);
    assert_eq!(
        superseded_history
            .allowed_validity
            .contains(&ItemValidity::Superseded),
        expected
    );
}

#[test]
fn explicit_history_intents_preserve_false_superseded_scope() {
    for intent in [
        ContextIntent::ExplainDecision,
        ContextIntent::ExploreHistory,
    ] {
        let p = plan(&request("t"), Some(intent)).unwrap();
        assert_eq!(p.intent_source, IntentSource::Explicit, "intent {intent:?}");
        assert_superseded_scope(&p, false);
    }
}

#[test]
fn keyword_history_fallback_preserves_false_superseded_scope() {
    let p = plan(&request("why was this decision replaced"), None).unwrap();
    assert_eq!(p.intent, ContextIntent::ExplainDecision);
    assert_eq!(p.intent_source, IntentSource::KeywordFallback);
    assert_superseded_scope(&p, false);
}

#[test]
fn default_history_fallback_preserves_false_superseded_scope() {
    let p = plan(&request("quarterly llama farming report"), None).unwrap();
    assert_eq!(p.intent, ContextIntent::ExploreHistory);
    assert_eq!(p.intent_source, IntentSource::DefaultFallback);
    assert_superseded_scope(&p, false);
}

#[test]
fn caller_opt_in_allows_superseded_across_plan_layers() {
    let mut opted_in = request("why was this decision replaced");
    opted_in.include_superseded = true;
    let p = plan(&opted_in, None).unwrap();
    assert_eq!(p.intent, ContextIntent::ExplainDecision);
    assert_eq!(p.intent_source, IntentSource::KeywordFallback);
    assert_superseded_scope(&p, true);
}

#[test]
fn non_history_intents_do_not_implicitly_expand_superseded_scope() {
    for intent in [
        ContextIntent::ResumeWork,
        ContextIntent::DebugFailure,
        ContextIntent::ApplyPreference,
        ContextIntent::ReviewChange,
    ] {
        let p = plan(&request("t"), Some(intent)).unwrap();
        assert!(!p.filters.include_superseded, "intent {intent:?}");
        assert!(!p.freshness_policy.include_superseded, "intent {intent:?}");
        assert!(
            p.channel_plans
                .iter()
                .all(|channel| !channel.allowed_validity.contains(&ItemValidity::Superseded)),
            "intent {intent:?}"
        );
    }
}

#[test]
fn apply_preference_requires_explicit_opt_in_for_superseded_conflict_history() {
    let default_plan = plan(&request("t"), Some(ContextIntent::ApplyPreference)).unwrap();
    let default_history = default_plan
        .channel_plans
        .iter()
        .find(|c| c.channel == RetrievalChannel::SupersededHistory)
        .unwrap();
    assert!(default_history.enabled);
    assert!(!default_history
        .allowed_validity
        .contains(&ItemValidity::Superseded));

    let mut opted_in = request("t");
    opted_in.include_superseded = true;
    let opted_in_plan = plan(&opted_in, Some(ContextIntent::ApplyPreference)).unwrap();
    assert!(opted_in_plan.filters.include_superseded);
    assert!(opted_in_plan.freshness_policy.include_superseded);
    let opted_in_history = opted_in_plan
        .channel_plans
        .iter()
        .find(|c| c.channel == RetrievalChannel::SupersededHistory)
        .unwrap();
    assert!(opted_in_history
        .allowed_validity
        .contains(&ItemValidity::Superseded));
}

// --- determinism / hashing ---------------------------------------------

#[test]
fn identical_requests_produce_identical_plans_and_hashes() {
    let a = plan(&request("resume the migration work"), None).unwrap();
    let b = plan(&request("resume the migration work"), None).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.plan_hash, b.plan_hash);
    assert_eq!(a.plan_hash.len(), 64);
}

#[test]
fn different_intents_produce_different_hashes() {
    let a = plan(&request("t"), Some(ContextIntent::ResumeWork)).unwrap();
    let b = plan(&request("t"), Some(ContextIntent::DebugFailure)).unwrap();
    assert_ne!(a.plan_hash, b.plan_hash);
}

#[test]
fn plan_json_round_trips() {
    let p = plan(&request("t"), Some(ContextIntent::ExplainDecision)).unwrap();
    let json = serde_json::to_string(&p).unwrap();
    let back: RetrievalPlan = serde_json::from_str(&json).unwrap();
    assert_eq!(p, back);
    // snake_case enum values are part of the contract.
    assert!(json.contains("\"intent\":\"explain_decision\""));
    assert!(json.contains("\"canonical_fts\""));
}

// --- validation --------------------------------------------------------

#[test]
fn invalid_requests_are_rejected() {
    let mut req = request("t");
    req.token_budget = 0;
    assert!(plan(&req, None).is_err());

    let mut req = request("t");
    req.project.key = " ".to_string();
    assert!(plan(&req, None).is_err());

    let mut req = request("t");
    req.schema_version = 99;
    assert!(plan(&req, None).is_err());
}
