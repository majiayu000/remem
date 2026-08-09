//! Deterministic v1 executor: applies a [`RetrievalPlan`] to caller-provided
//! candidates by reusing the SessionStart relevance selector, enforces
//! section and total token budgets, and emits an audited [`ContextBundle`].
//!
//! v1 has no DB access; wiring the executor to the SessionStart loaders is
//! follow-up work on GH-932.

use std::collections::HashMap;

use crate::context::{
    build_sessionstart_relevance_plan, RelevanceCandidate, RelevanceSection,
    SessionStartRelevancePlan,
};

use super::audit::AuditBuilder;
use crate::retrieval_router::{AbstentionMode, RetrievalPlan};

use super::domain::{
    ChannelKind, ContextBundle, ContextItem, DegradedMode, ItemValidity, SourceKind, TrustClass,
    CONTEXT_BUNDLE_SCHEMA_VERSION,
};
use super::policy::{
    estimate_item_tokens, validate_plan, REASON_BELOW_TRUST_FLOOR, REASON_BRANCH_SCOPE_MISMATCH,
    REASON_CANONICAL_LOAD_FAILED, REASON_CANONICAL_ONLY_DEGRADED, REASON_CHANNEL_ITEM_LIMIT,
    REASON_CHANNEL_TOKEN_BUDGET, REASON_PLAN_BLOCKED, REASON_POISONING_GATE,
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
    /// Candidates rejected by the canonical loader's poisoning gate. Their
    /// title/text never enter the returned bundle, but their redacted identity
    /// remains in the audit so the endpoint accounts for every loaded row.
    pub poisoning_drops: Vec<ContextItem>,
    /// Safe canonical rows intentionally omitted by an upstream canonical
    /// selector. These are audit-only and never enter a returned section.
    pub preselection_drops: Vec<PreselectionDrop>,
    pub enrichment_available: bool,
}

#[derive(Debug, Clone)]
pub struct PreselectionDrop {
    pub item: ContextItem,
    pub reason: String,
}

/// SessionStart's compatibility renderer owns exact character and item
/// boundaries. The generic executor keeps strict token enforcement, while the
/// renderer integration may defer those final budget decisions and seal the
/// bundle after byte-compatible rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BudgetEnforcement {
    Strict,
    DeferToRenderer,
}

pub(crate) struct ExecutionTrace {
    pub bundle: ContextBundle,
    pub relevance_plan: SessionStartRelevancePlan,
}

/// Execute a plan over the provided candidates.
///
/// Deterministic: same plan + same inputs always produce the same bundle.
/// An invalid plan (schema/policy/scope) produces a `blocked` bundle whose
/// audit drops every candidate; it never partially executes.
pub fn execute(plan: &RetrievalPlan, inputs: &ExecutorInputs) -> ContextBundle {
    execute_with_trace(plan, inputs, BudgetEnforcement::Strict).bundle
}

pub(crate) fn execute_with_trace(
    plan: &RetrievalPlan,
    inputs: &ExecutorInputs,
    budget_enforcement: BudgetEnforcement,
) -> ExecutionTrace {
    if let Err(error) = validate_plan(plan) {
        return ExecutionTrace {
            bundle: blocked_bundle(plan, inputs, &error.to_string()),
            relevance_plan: SessionStartRelevancePlan::disabled(&[]),
        };
    }
    let degraded_mode = if inputs.enrichment_available {
        DegradedMode::Full
    } else {
        DegradedMode::CanonicalOnly
    };

    let mut audit = AuditBuilder::default();
    for item in &inputs.poisoning_drops {
        audit.dropped(item, REASON_POISONING_GATE);
    }
    for dropped in &inputs.preselection_drops {
        audit.dropped(&dropped.item, &dropped.reason);
    }
    let mut in_scope: Vec<&ContextItem> = Vec::new();
    for item in &inputs.candidates {
        match scope_drop_reason(plan, degraded_mode, item) {
            Some(reason) => audit.dropped(item, reason),
            None => in_scope.push(item),
        }
    }

    let (relevance, relevance_plan) = relevance_decisions(plan, &in_scope, &mut audit);
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
    match budget_enforcement {
        BudgetEnforcement::Strict => {
            order_relevance_governed_survivors(plan, &relevance_plan, &mut survivors);
            apply_budgets(plan, degraded_mode, &survivors, &mut bundle, &mut audit)
        }
        BudgetEnforcement::DeferToRenderer => {
            select_for_renderer(plan, &survivors, &mut bundle, &mut audit)
        }
    }
    if plan.abstention_policy.mode == AbstentionMode::OnLowEvidence
        && audit.selected_count() < plan.abstention_policy.min_selected_items
    {
        clear_bundle_sections(&mut bundle);
        audit.abstain();
    }
    bundle.audit = audit.finalize(plan, degraded_mode);
    ExecutionTrace {
        bundle,
        relevance_plan,
    }
}

/// Section limits must consume relevance-selected rows in relevance order,
/// not in the canonical loader's incidental row order. The stable sort keeps
/// non-governed channels and disabled relevance plans byte-for-byte unchanged.
fn order_relevance_governed_survivors(
    plan: &RetrievalPlan,
    relevance_plan: &SessionStartRelevancePlan,
    survivors: &mut [&ContextItem],
) {
    let ranks = relevance_plan
        .selected_keys()
        .iter()
        .enumerate()
        .map(|(rank, key)| (key.as_str(), rank))
        .collect::<HashMap<_, _>>();
    survivors.sort_by_key(|item| {
        if channel_relevance_governed(plan, item.channel) {
            ranks
                .get(item.stable_key.as_str())
                .copied()
                .unwrap_or(usize::MAX)
        } else {
            usize::MAX
        }
    });
}

fn scope_drop_reason(
    plan: &RetrievalPlan,
    degraded_mode: DegradedMode,
    item: &ContextItem,
) -> Option<&'static str> {
    if item.trust == TrustClass::Quarantined {
        return Some(REASON_QUARANTINED_TRUST);
    }
    if trust_rank(item.trust) < trust_rank(plan.trust_policy.minimum_trust) {
        return Some(REASON_BELOW_TRUST_FLOOR);
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

fn trust_rank(trust: TrustClass) -> u8 {
    match trust {
        TrustClass::Quarantined => 0,
        TrustClass::Standard => 1,
        TrustClass::Trusted => 2,
    }
}

fn clear_bundle_sections(bundle: &mut ContextBundle) {
    bundle.preferences.clear();
    bundle.failure_lessons.clear();
    bundle.current_truth.clear();
    bundle.workstreams.clear();
    bundle.memory_index.clear();
    bundle.recent_sessions.clear();
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
) -> (
    HashMap<&'a str, (bool, &'static str)>,
    SessionStartRelevancePlan,
) {
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
    (decisions, relevance_plan)
}

/// Preserve scope/relevance decisions in the bundle, but leave exact item,
/// section-character, and total-character enforcement to the established
/// SessionStart renderer. The render path seals the returned bundle to the
/// identities that survived those exact boundaries before it can be exposed
/// to any downstream consumer.
fn select_for_renderer(
    plan: &RetrievalPlan,
    survivors: &[&ContextItem],
    bundle: &mut ContextBundle,
    audit: &mut AuditBuilder,
) {
    for item in survivors {
        let governed = channel_relevance_governed(plan, item.channel);
        let reason = if governed {
            REASON_SELECTED_RELEVANCE
        } else {
            REASON_SELECTED_CHANNEL
        };
        audit.selected(item, reason);
        bundle.section_mut(item.channel).push((*item).clone());
    }
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
            let tokens = estimate_item_tokens(item);
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
    for item in &inputs.poisoning_drops {
        audit.dropped(item, REASON_POISONING_GATE);
    }
    for dropped in &inputs.preselection_drops {
        audit.dropped(&dropped.item, &dropped.reason);
    }
    audit.set_truncation_reason(REASON_PLAN_BLOCKED);
    let mut bundle = empty_bundle(plan, DegradedMode::Blocked);
    bundle.audit = audit.finalize(plan, DegradedMode::Blocked);
    bundle
}

/// A `Blocked` bundle for a failure that happened before any candidate
/// existed — a canonical load error, most importantly.
///
/// The bundle is empty and its audit records `reason` as the truncation
/// reason, so a caller cannot mistake "canonical data could not be read"
/// for "this project has no memory".
pub fn blocked_before_load(plan: &RetrievalPlan, reason: &str) -> ContextBundle {
    crate::log::error(
        "context-bundle",
        &format!("canonical load failed; emitting blocked bundle: {reason}"),
    );
    let mut audit = AuditBuilder::default();
    audit.set_truncation_reason(REASON_CANONICAL_LOAD_FAILED);
    let mut bundle = empty_bundle(plan, DegradedMode::Blocked);
    bundle.audit = audit.finalize(plan, DegradedMode::Blocked);
    bundle
}
