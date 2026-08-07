//! Deterministic v1 executor: applies a [`RetrievalPlan`] to caller-provided
//! candidates by reusing the SessionStart relevance selector, enforces
//! section and total token budgets, and emits an audited [`ContextBundle`].
//!
//! v1 has no DB access; wiring the executor to the SessionStart loaders is
//! follow-up work on GH-932.

use std::collections::HashMap;

use crate::context::{build_sessionstart_relevance_plan, RelevanceCandidate, RelevanceSection};

use super::audit::AuditBuilder;
use crate::retrieval_router::RetrievalPlan;

use super::domain::{
    ChannelKind, ContextBundle, ContextItem, DegradedMode, ItemValidity, SourceKind, TrustClass,
    CONTEXT_BUNDLE_SCHEMA_VERSION,
};
use super::policy::{
    estimate_tokens, validate_plan, REASON_BRANCH_SCOPE_MISMATCH, REASON_CANONICAL_ONLY_DEGRADED,
    REASON_CHANNEL_ITEM_LIMIT, REASON_CHANNEL_TOKEN_BUDGET, REASON_PLAN_BLOCKED,
    REASON_PROJECT_SCOPE_MISMATCH, REASON_QUARANTINED_TRUST, REASON_SELECTED_CHANNEL,
    REASON_SELECTED_RELEVANCE, REASON_SUPERSEDED_EXCLUDED, REASON_TOTAL_TOKEN_BUDGET,
};

/// Candidate inputs for one execution. `enrichment_available = false`
/// degrades the bundle to `canonical_only`: generated and graph-derived
/// candidates are dropped instead of being served without their backing
/// enrichment stack.
#[derive(Debug, Clone)]
pub struct ExecutorInputs {
    pub candidates: Vec<ContextItem>,
    pub enrichment_available: bool,
}

/// Execute a plan over the provided candidates.
///
/// Deterministic: same plan + same inputs always produce the same bundle.
/// An invalid plan (schema/policy/scope) produces a `blocked` bundle whose
/// audit drops every candidate; it never partially executes.
pub fn execute(plan: &RetrievalPlan, inputs: &ExecutorInputs) -> ContextBundle {
    if let Err(error) = validate_plan(plan) {
        return blocked_bundle(plan, inputs, &error.to_string());
    }
    let degraded_mode = if inputs.enrichment_available {
        DegradedMode::Full
    } else {
        DegradedMode::CanonicalOnly
    };

    let mut audit = AuditBuilder::default();
    let mut in_scope: Vec<&ContextItem> = Vec::new();
    for item in &inputs.candidates {
        match scope_drop_reason(plan, degraded_mode, item) {
            Some(reason) => audit.dropped(item, reason),
            None => in_scope.push(item),
        }
    }

    let relevance = relevance_decisions(plan, &in_scope, &mut audit);
    let mut survivors: Vec<&ContextItem> = Vec::new();
    for item in in_scope {
        let governed = channel_relevance_governed(plan, item.channel);
        if governed {
            match relevance.get(item.stable_key.as_str()) {
                Some(&(true, _)) | None => survivors.push(item),
                Some(&(false, drop_reason)) => audit.dropped(item, drop_reason),
            }
        } else {
            survivors.push(item);
        }
    }

    let mut bundle = empty_bundle(plan, degraded_mode);
    apply_budgets(plan, degraded_mode, &survivors, &mut bundle, &mut audit);
    bundle.audit = audit.finalize(plan, degraded_mode);
    bundle
}

fn scope_drop_reason(
    plan: &RetrievalPlan,
    degraded_mode: DegradedMode,
    item: &ContextItem,
) -> Option<&'static str> {
    if item.trust == TrustClass::Quarantined {
        return Some(REASON_QUARANTINED_TRUST);
    }
    if let Some(project) = &item.project {
        if project != &plan.filters.project {
            return Some(REASON_PROJECT_SCOPE_MISMATCH);
        }
    }
    if let (Some(item_branch), Some(plan_branch)) = (&item.branch, &plan.filters.branch) {
        if item_branch != plan_branch {
            return Some(REASON_BRANCH_SCOPE_MISMATCH);
        }
    }
    if item.validity == ItemValidity::Superseded && !plan.filters.include_superseded {
        return Some(REASON_SUPERSEDED_EXCLUDED);
    }
    if degraded_mode == DegradedMode::CanonicalOnly && item.source_kind != SourceKind::Canonical {
        return Some(REASON_CANONICAL_ONLY_DEGRADED);
    }
    None
}

fn channel_relevance_governed(plan: &RetrievalPlan, channel: ChannelKind) -> bool {
    plan.output_sections
        .iter()
        .find(|planned| planned.channel == channel)
        .is_some_and(|planned| planned.relevance_governed)
}

fn relevance_section(channel: ChannelKind) -> Option<RelevanceSection> {
    match channel {
        ChannelKind::Lessons => Some(RelevanceSection::Lessons),
        ChannelKind::MemoryIndex => Some(RelevanceSection::MemoryIndex),
        ChannelKind::Sessions => Some(RelevanceSection::Sessions),
        ChannelKind::Preferences | ChannelKind::Core | ChannelKind::Workstreams => None,
    }
}

/// Reuse the SessionStart relevance selector for the governed channels.
/// Returns `stable_key -> (selected, drop_reason)`; drop reasons are the
/// SessionStart reason strings.
fn relevance_decisions<'a>(
    plan: &RetrievalPlan,
    in_scope: &[&'a ContextItem],
    audit: &mut AuditBuilder,
) -> HashMap<&'a str, (bool, &'static str)> {
    let candidates: Vec<RelevanceCandidate> = in_scope
        .iter()
        .filter(|item| channel_relevance_governed(plan, item.channel))
        .filter_map(|item| {
            relevance_section(item.channel).map(|section| RelevanceCandidate {
                stable_key: item.stable_key.clone(),
                section,
                text: format!("{} {}", item.title, item.text),
            })
        })
        .collect();
    let relevance_plan = build_sessionstart_relevance_plan(
        plan.relevance_query.as_deref(),
        plan.relevance_k as usize,
        &candidates,
    );
    let mut decisions = HashMap::new();
    for item in in_scope {
        let Some(decision) = relevance_plan.decision(&item.stable_key) else {
            continue;
        };
        audit.record_score(&item.stable_key, decision.score);
        decisions.insert(
            item.stable_key.as_str(),
            (decision.selected, decision.drop_reason.unwrap_or("dropped")),
        );
    }
    decisions
}

/// Enforce per-channel item limits, per-channel token budgets, and the
/// total token budget in the fixed [`ChannelKind::ORDERED`] order.
fn apply_budgets(
    plan: &RetrievalPlan,
    _degraded_mode: DegradedMode,
    survivors: &[&ContextItem],
    bundle: &mut ContextBundle,
    audit: &mut AuditBuilder,
) {
    let mut total_tokens: u32 = 0;
    let total_budget = plan.section_budgets.total_tokens;
    for channel in ChannelKind::ORDERED {
        let item_limit = plan
            .output_sections
            .iter()
            .find(|planned| planned.channel == channel)
            .map(|planned| planned.item_limit)
            .unwrap_or(0);
        let channel_budget = plan.section_budgets.for_channel(channel);
        let governed = channel_relevance_governed(plan, channel);
        let mut channel_tokens: u32 = 0;
        let mut channel_count: u32 = 0;
        for item in survivors.iter().filter(|item| item.channel == channel) {
            let tokens = estimate_tokens(&item.text);
            if channel_count >= item_limit {
                audit.dropped(item, REASON_CHANNEL_ITEM_LIMIT);
                continue;
            }
            if channel_tokens + tokens > channel_budget {
                audit.dropped(item, REASON_CHANNEL_TOKEN_BUDGET);
                continue;
            }
            if total_tokens + tokens > total_budget {
                audit.dropped(item, REASON_TOTAL_TOKEN_BUDGET);
                audit.set_truncation_reason(REASON_TOTAL_TOKEN_BUDGET);
                continue;
            }
            channel_count += 1;
            channel_tokens += tokens;
            total_tokens += tokens;
            let reason = if governed {
                REASON_SELECTED_RELEVANCE
            } else {
                REASON_SELECTED_CHANNEL
            };
            audit.selected(item, reason);
            bundle.section_mut(channel).push((*item).clone());
        }
    }
}

fn empty_bundle(plan: &RetrievalPlan, degraded_mode: DegradedMode) -> ContextBundle {
    ContextBundle {
        schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
        plan_hash: plan.plan_hash.clone(),
        degraded_mode,
        preferences: Vec::new(),
        failure_lessons: Vec::new(),
        current_truth: Vec::new(),
        workstreams: Vec::new(),
        memory_index: Vec::new(),
        recent_sessions: Vec::new(),
        audit: super::domain::ContextAudit {
            schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
            policy_version: plan.policy_version.clone(),
            relevance_policy_version: plan.relevance_policy_version.clone(),
            plan_hash: plan.plan_hash.clone(),
            degraded_mode,
            candidates_considered: 0,
            selected_count: 0,
            dropped_count: 0,
            token_estimate: 0,
            token_budget: plan.section_budgets.total_tokens,
            truncation_reason: None,
            entries: Vec::new(),
        },
    }
}

fn blocked_bundle(plan: &RetrievalPlan, inputs: &ExecutorInputs, error: &str) -> ContextBundle {
    crate::log::error(
        "context-bundle",
        &format!("plan validation failed; emitting blocked bundle: {error}"),
    );
    let mut audit = AuditBuilder::default();
    for item in &inputs.candidates {
        audit.dropped(item, REASON_PLAN_BLOCKED);
    }
    audit.set_truncation_reason(REASON_PLAN_BLOCKED);
    let mut bundle = empty_bundle(plan, DegradedMode::Blocked);
    bundle.audit = audit.finalize(plan, DegradedMode::Blocked);
    bundle
}
