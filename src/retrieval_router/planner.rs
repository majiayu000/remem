//! Deterministic per-intent plan compilation for the Retrieval Router
//! (GH-934). The plan is a pure function of the request, the optional
//! explicit intent, and the compiled policy: no clock reads, no env
//! access, no randomness, no LLM. Identical inputs always produce
//! identical plan JSON and `plan_hash`.

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::context_bundle::{
    validate_request, AgentRole, ContextFilters, ContextIntent, ContextRequest, ItemValidity,
    RiskClass, TrustClass,
};

use super::domain::{
    AbstentionMode, AbstentionPolicy, ChannelDegradation, ChannelPlan, FreshnessPolicy,
    RerankFallback, RerankPolicy, RetrievalChannel, RetrievalPlan, TrustPolicy,
    RETRIEVAL_PLAN_SCHEMA_VERSION,
};
use super::intent::resolve_intent;

/// Bump when the intent mapping tables or policy adjustments change.
pub const RETRIEVAL_ROUTER_POLICY_VERSION: &str = "retrieval_router_v1";

// Reason codes for deterministic policy adjustments.
const REASON_HIGH_RISK_TRUSTED_ONLY: &str = "high_risk_trusted_only";
const REASON_HIGH_RISK_ENRICHMENT_DISABLED: &str = "high_risk_enrichment_disabled";
const REASON_HIGH_RISK_CANONICAL_TOP1: &str = "high_risk_canonical_evidence_top1";
const REASON_HIGH_RISK_ABSTAIN_ON_LOW_EVIDENCE: &str = "high_risk_abstain_on_low_evidence";
const REASON_REVIEWER_CONSTRAINTS_ENABLED: &str = "reviewer_constraints_enabled";

// Baseline mechanical channels shared by every intent.
const BASELINE_FTS: ChannelSpec = ChannelSpec {
    channel: RetrievalChannel::CanonicalFts,
    weight: 0.8,
    candidate_limit: 20,
    max_contribution: 8,
    timeout_ms: 300,
    degradation: ChannelDegradation::FailClosed,
    allow_history: false,
};
const BASELINE_VECTOR: ChannelSpec = ChannelSpec {
    channel: RetrievalChannel::CanonicalVector,
    weight: 0.7,
    candidate_limit: 20,
    max_contribution: 8,
    timeout_ms: 500,
    degradation: ChannelDegradation::SkipChannel,
    allow_history: false,
};
/// Generated enrichment is always a capped secondary signal with skip
/// degradation; it never gets canonical-level weight or contribution.
const BASELINE_ENRICHMENT: ChannelSpec = ChannelSpec {
    channel: RetrievalChannel::GeneratedEnrichment,
    weight: 0.3,
    candidate_limit: 10,
    max_contribution: 2,
    timeout_ms: 300,
    degradation: ChannelDegradation::SkipChannel,
    allow_history: false,
};

struct ChannelSpec {
    channel: RetrievalChannel,
    weight: f64,
    candidate_limit: u32,
    max_contribution: u32,
    timeout_ms: u32,
    degradation: ChannelDegradation,
    /// Whether the channel may return stale/superseded validity.
    allow_history: bool,
}

impl ChannelSpec {
    const fn priority(
        channel: RetrievalChannel,
        weight: f64,
        candidate_limit: u32,
        max_contribution: u32,
    ) -> Self {
        ChannelSpec {
            channel,
            weight,
            candidate_limit,
            max_contribution,
            timeout_ms: 300,
            degradation: ChannelDegradation::SkipChannel,
            allow_history: false,
        }
    }

    const fn history(
        channel: RetrievalChannel,
        weight: f64,
        candidate_limit: u32,
        max_contribution: u32,
    ) -> Self {
        ChannelSpec {
            channel,
            weight,
            candidate_limit,
            max_contribution,
            timeout_ms: 300,
            degradation: ChannelDegradation::SkipChannel,
            allow_history: true,
        }
    }
}

/// Intent-priority channels layered on top of the shared baseline,
/// following the GH-934 per-intent priority lists. Locked by unit tests.
fn intent_priority_channels(intent: ContextIntent) -> Vec<ChannelSpec> {
    use RetrievalChannel as C;
    match intent {
        // active workstreams, recent verified session outcomes,
        // current decisions, blockers / next actions
        ContextIntent::ResumeWork => vec![
            ChannelSpec::priority(C::Workstreams, 1.0, 5, 5),
            ChannelSpec::priority(C::SessionOutcomes, 0.9, 5, 3),
            ChannelSpec::priority(C::Decisions, 0.7, 5, 3),
            ChannelSpec::priority(C::Temporal, 0.5, 10, 3),
        ],
        // decision memories, superseded decisions, git/PR evidence,
        // benchmark evidence, temporal channel
        ContextIntent::ExplainDecision => vec![
            ChannelSpec::priority(C::Decisions, 1.0, 10, 5),
            ChannelSpec::history(C::SupersededHistory, 0.9, 10, 5),
            ChannelSpec::priority(C::GitEvidence, 0.8, 10, 5),
            ChannelSpec::priority(C::BenchmarkEvidence, 0.6, 5, 3),
            ChannelSpec::history(C::Temporal, 0.5, 10, 3),
        ],
        // failure lessons, error signatures, affected files/entities,
        // failed attempts, related bugfix memories
        ContextIntent::DebugFailure => vec![
            ChannelSpec::priority(C::FailureLessons, 1.0, 10, 5),
            ChannelSpec::priority(C::EntityGraph, 0.8, 10, 5),
            ChannelSpec::priority(C::GitEvidence, 0.6, 5, 3),
            ChannelSpec::priority(C::SessionOutcomes, 0.5, 5, 2),
        ],
        // current user/project preference, explicit scope,
        // compiled-rule provenance, conflicts / suppressions
        ContextIntent::ApplyPreference => vec![
            ChannelSpec::priority(C::Preferences, 1.0, 10, 5),
            ChannelSpec::priority(C::Constraints, 0.6, 5, 3),
            ChannelSpec::history(C::SupersededHistory, 0.5, 5, 2),
        ],
        // architecture constraints, negative constraints, prior
        // regressions, branch-specific decisions, test/release policy
        ContextIntent::ReviewChange => vec![
            ChannelSpec::priority(C::Constraints, 1.0, 10, 5),
            ChannelSpec::priority(C::FailureLessons, 0.8, 10, 5),
            ChannelSpec::priority(C::Decisions, 0.7, 10, 5),
            ChannelSpec::priority(C::GitEvidence, 0.6, 5, 3),
        ],
        // timeline, session summaries, historical/superseded claims,
        // graph expansion. Also the conservative fallback / generic
        // policy for SessionStart and unclassified tasks.
        ContextIntent::ExploreHistory | ContextIntent::SessionStart => vec![
            ChannelSpec::history(C::Temporal, 1.0, 15, 8),
            ChannelSpec::priority(C::SessionOutcomes, 0.9, 10, 5),
            ChannelSpec::history(C::SupersededHistory, 0.7, 10, 5),
            ChannelSpec::priority(C::GraphExpansion, 0.5, 10, 3),
        ],
    }
}

/// Which intents enable second-stage rerank (GH-851 owns the mechanism;
/// the router only selects participation, pool sizes, and fallback).
fn rerank_policy_for(intent: ContextIntent) -> RerankPolicy {
    let enabled = matches!(
        intent,
        ContextIntent::ExplainDecision | ContextIntent::DebugFailure | ContextIntent::ReviewChange
    );
    RerankPolicy {
        enabled,
        candidate_pool: if enabled { 50 } else { 0 },
        output_k: if enabled { 10 } else { 0 },
        timeout_fallback: RerankFallback::SkipRerank,
        require_canonical_evidence_top1: false,
    }
}

fn freshness_policy_for(intent: ContextIntent, request: &ContextRequest) -> FreshnessPolicy {
    FreshnessPolicy {
        prefer_current: true,
        include_superseded: request.include_superseded,
        max_age_days: match intent {
            ContextIntent::ResumeWork => Some(30),
            _ => None,
        },
    }
}

/// Compile a deterministic retrieval plan for the request.
///
/// `explicit_intent` comes from the caller (API field, MCP tool
/// parameter, or CLI flag) and always wins over keyword resolution.
pub fn plan(
    request: &ContextRequest,
    explicit_intent: Option<ContextIntent>,
) -> Result<RetrievalPlan> {
    validate_request(request)?;
    let resolved = resolve_intent(explicit_intent, &request.task);
    let mut reason_codes = vec![resolved.reason_code.clone()];

    let freshness_policy = freshness_policy_for(resolved.intent, request);
    let mut channel_plans = compile_channels(resolved.intent, freshness_policy.include_superseded);
    let mut rerank_policy = rerank_policy_for(resolved.intent);
    let mut trust_policy = TrustPolicy {
        minimum_trust: TrustClass::Standard,
        allow_quarantined: false,
    };
    let mut abstention_policy = AbstentionPolicy {
        mode: AbstentionMode::Never,
        min_selected_items: 0,
    };

    // Deterministic risk adjustments: high-risk requests tighten trust,
    // drop the generated-enrichment signal, require canonical evidence
    // at top 1, and abstain instead of padding weak evidence.
    if request.risk == RiskClass::High {
        trust_policy.minimum_trust = TrustClass::Trusted;
        reason_codes.push(REASON_HIGH_RISK_TRUSTED_ONLY.to_string());
        for cp in channel_plans.iter_mut() {
            if cp.channel == RetrievalChannel::GeneratedEnrichment && cp.enabled {
                disable_channel(cp);
                reason_codes.push(REASON_HIGH_RISK_ENRICHMENT_DISABLED.to_string());
            }
        }
        rerank_policy.require_canonical_evidence_top1 = true;
        reason_codes.push(REASON_HIGH_RISK_CANONICAL_TOP1.to_string());
        abstention_policy = AbstentionPolicy {
            mode: AbstentionMode::OnLowEvidence,
            min_selected_items: 1,
        };
        reason_codes.push(REASON_HIGH_RISK_ABSTAIN_ON_LOW_EVIDENCE.to_string());
    }

    // Deterministic role adjustment: reviewers always see architecture /
    // negative constraints even when the intent mapping leaves them off.
    if request.role == AgentRole::Reviewer {
        for cp in channel_plans.iter_mut() {
            if cp.channel == RetrievalChannel::Constraints && !cp.enabled {
                *cp = channel_plan_from_spec(
                    &ChannelSpec::priority(RetrievalChannel::Constraints, 0.6, 5, 3),
                    freshness_policy.include_superseded,
                );
                reason_codes.push(REASON_REVIEWER_CONSTRAINTS_ENABLED.to_string());
            }
        }
    }

    let mut plan = RetrievalPlan {
        schema_version: RETRIEVAL_PLAN_SCHEMA_VERSION,
        policy_version: RETRIEVAL_ROUTER_POLICY_VERSION.to_string(),
        intent: resolved.intent,
        intent_source: resolved.source,
        role: request.role,
        risk: request.risk,
        reason_codes,
        channel_plans,
        filters: ContextFilters {
            project: request.project.key.clone(),
            branch: request.branch.clone(),
            include_superseded: freshness_policy.include_superseded,
            as_of_epoch: request.as_of_epoch,
        },
        rerank_policy,
        trust_policy,
        freshness_policy,
        token_budget: request.token_budget,
        abstention_policy,
        plan_hash: String::new(),
    };
    plan.plan_hash = plan_content_hash(&plan)?;
    Ok(plan)
}

/// Build one `ChannelPlan` per known channel (enabled or disabled) in
/// [`RetrievalChannel::ORDERED`] order.
fn compile_channels(intent: ContextIntent, include_superseded: bool) -> Vec<ChannelPlan> {
    let mut specs = vec![BASELINE_FTS, BASELINE_VECTOR, BASELINE_ENRICHMENT];
    specs.extend(intent_priority_channels(intent));
    RetrievalChannel::ORDERED
        .iter()
        .map(|channel| {
            specs
                .iter()
                .find(|spec| spec.channel == *channel)
                .map(|spec| channel_plan_from_spec(spec, include_superseded))
                .unwrap_or_else(|| disabled_channel_plan(*channel))
        })
        .collect()
}

fn channel_plan_from_spec(spec: &ChannelSpec, include_superseded: bool) -> ChannelPlan {
    let mut allowed_validity = vec![ItemValidity::Current];
    if spec.allow_history {
        allowed_validity.push(ItemValidity::Stale);
        if include_superseded {
            allowed_validity.push(ItemValidity::Superseded);
        }
    }
    ChannelPlan {
        channel: spec.channel,
        enabled: true,
        candidate_limit: spec.candidate_limit,
        weight: spec.weight,
        required_trust: TrustClass::Standard,
        allowed_validity,
        max_contribution: spec.max_contribution,
        timeout_ms: spec.timeout_ms,
        degradation: spec.degradation,
    }
}

fn disabled_channel_plan(channel: RetrievalChannel) -> ChannelPlan {
    ChannelPlan {
        channel,
        enabled: false,
        candidate_limit: 0,
        weight: 0.0,
        required_trust: TrustClass::Standard,
        allowed_validity: Vec::new(),
        max_contribution: 0,
        timeout_ms: 0,
        degradation: ChannelDegradation::SkipChannel,
    }
}

fn disable_channel(cp: &mut ChannelPlan) {
    *cp = disabled_channel_plan(cp.channel);
}

/// SHA-256 hex over the canonical serde JSON of the plan with an empty
/// `plan_hash` field — the same convention as `ContextPlan.plan_hash`
/// (GH-932). serde JSON struct field order is declaration order, so the
/// byte stream is stable for a fixed schema version.
fn plan_content_hash(plan: &RetrievalPlan) -> Result<String> {
    let mut hashable = plan.clone();
    hashable.plan_hash = String::new();
    let canonical = serde_json::to_string(&hashable)?;
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}
