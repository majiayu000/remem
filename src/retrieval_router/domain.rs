//! Versioned DTOs for the Retrieval Router v1 plan contract (GH-934).
//!
//! `RetrievalPlan` carries `schema_version` so later revisions can break
//! the shape explicitly. Enums serialize as snake_case serde JSON.
//! Scope filters, item validity, trust classes, roles, and risk classes
//! are reused from `crate::context_bundle` so the router and the bundle
//! contract cannot drift apart.

use serde::{Deserialize, Serialize};

use crate::context_bundle::{
    AgentRole, ContextFilters, ContextIntent, ItemValidity, PlannedChannel, RiskClass,
    SectionBudgets, TrustClass,
};

/// Version of the RetrievalPlan JSON shape.
pub const RETRIEVAL_PLAN_SCHEMA_VERSION: u32 = 1;

/// Retrieval channels the v1 router can enable. Mechanical channels
/// (fts/vector/enrichment/graph) and evidence-type channels (decisions,
/// git evidence, lessons, ...) share one enum so a plan can state, per
/// intent, exactly which evidence sources participate and with what caps.
///
/// `GeneratedEnrichment` is the write-side enrichment projection signal
/// (GH-850/928). It is always a separate channel with its own
/// contribution cap so generated text never receives canonical FTS +
/// vector double weighting and keeps its attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalChannel {
    CanonicalFts,
    CanonicalVector,
    GeneratedEnrichment,
    EntityGraph,
    GraphExpansion,
    Temporal,
    Workstreams,
    SessionOutcomes,
    Decisions,
    SupersededHistory,
    GitEvidence,
    BenchmarkEvidence,
    FailureLessons,
    Preferences,
    Constraints,
}

impl RetrievalChannel {
    /// Deterministic plan order; every plan lists all channels in this
    /// order with `enabled` true/false so debug output shows both the
    /// selected and the disabled set.
    pub const ORDERED: [RetrievalChannel; 15] = [
        RetrievalChannel::CanonicalFts,
        RetrievalChannel::CanonicalVector,
        RetrievalChannel::GeneratedEnrichment,
        RetrievalChannel::EntityGraph,
        RetrievalChannel::GraphExpansion,
        RetrievalChannel::Temporal,
        RetrievalChannel::Workstreams,
        RetrievalChannel::SessionOutcomes,
        RetrievalChannel::Decisions,
        RetrievalChannel::SupersededHistory,
        RetrievalChannel::GitEvidence,
        RetrievalChannel::BenchmarkEvidence,
        RetrievalChannel::FailureLessons,
        RetrievalChannel::Preferences,
        RetrievalChannel::Constraints,
    ];

    /// Stable snake_case name (matches the serde rename).
    pub fn name(&self) -> &'static str {
        match self {
            RetrievalChannel::CanonicalFts => "canonical_fts",
            RetrievalChannel::CanonicalVector => "canonical_vector",
            RetrievalChannel::GeneratedEnrichment => "generated_enrichment",
            RetrievalChannel::EntityGraph => "entity_graph",
            RetrievalChannel::GraphExpansion => "graph_expansion",
            RetrievalChannel::Temporal => "temporal",
            RetrievalChannel::Workstreams => "workstreams",
            RetrievalChannel::SessionOutcomes => "session_outcomes",
            RetrievalChannel::Decisions => "decisions",
            RetrievalChannel::SupersededHistory => "superseded_history",
            RetrievalChannel::GitEvidence => "git_evidence",
            RetrievalChannel::BenchmarkEvidence => "benchmark_evidence",
            RetrievalChannel::FailureLessons => "failure_lessons",
            RetrievalChannel::Preferences => "preferences",
            RetrievalChannel::Constraints => "constraints",
        }
    }
}

/// What the executor must do when a channel times out or errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelDegradation {
    /// Drop the channel's contribution and continue with the rest.
    SkipChannel,
    /// The channel is required base evidence; failure fails the plan
    /// closed (abstain rather than answer from weaker signals).
    FailClosed,
}

/// Per-channel execution parameters compiled into the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelPlan {
    pub channel: RetrievalChannel,
    pub enabled: bool,
    /// Maximum candidates fetched from this channel.
    pub candidate_limit: u32,
    /// Fusion weight relative to the other enabled channels.
    pub weight: f64,
    /// Minimum trust class a candidate needs to enter fusion.
    pub required_trust: TrustClass,
    /// Validity states the channel may return.
    pub allowed_validity: Vec<ItemValidity>,
    /// Hard cap on items this channel may contribute to the final result.
    pub max_contribution: u32,
    pub timeout_ms: u32,
    pub degradation: ChannelDegradation,
}

/// Router-side rerank selection. GH-851 owns the model and execution
/// mechanics; the router only decides participation and fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerankPolicy {
    pub enabled: bool,
    /// Candidate pool size handed to the reranker (candidate N).
    pub candidate_pool: u32,
    /// Results kept after rerank (output k).
    pub output_k: u32,
    pub timeout_fallback: RerankFallback,
    /// High-risk requests must not let a generated-only item reach top 1;
    /// the top result needs canonical evidence.
    pub require_canonical_evidence_top1: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RerankFallback {
    /// On rerank timeout/error keep the fusion order unchanged.
    SkipRerank,
}

/// v1 trust policy placeholder: a global floor. Per-channel floors live
/// on `ChannelPlan::required_trust`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustPolicy {
    pub minimum_trust: TrustClass,
    /// Always false in v1; quarantined memories never enter retrieval.
    pub allow_quarantined: bool,
}

/// v1 freshness policy placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreshnessPolicy {
    /// Prefer `current` validity over stale on ties.
    pub prefer_current: bool,
    /// Whether superseded memories may appear at all (history intents).
    pub include_superseded: bool,
    /// Optional recency window in days for recency-driven intents.
    pub max_age_days: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbstentionMode {
    /// Return whatever evidence exists.
    Never,
    /// Abstain (return nothing) when selected evidence is below the
    /// minimum instead of padding with weak matches.
    OnLowEvidence,
}

/// v1 abstention policy placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbstentionPolicy {
    pub mode: AbstentionMode,
    pub min_selected_items: u32,
}

/// How the intent on a plan was determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentSource {
    /// Caller passed the intent explicitly (API / CLI flag).
    Explicit,
    /// Deterministic keyword rules matched the task text.
    KeywordFallback,
    /// Nothing matched; conservative generic fallback.
    DefaultFallback,
}

/// Resolved intent plus provenance for audit output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedIntent {
    pub intent: ContextIntent,
    pub source: IntentSource,
    pub reason_code: String,
}

/// The compiled, deterministic retrieval plan (GH-934 / GH-932 v1).
///
/// The plan spans two distinct layers and they must not be conflated:
///
/// - `channel_plans` is the **retrieval-source** side: where candidates
///   are fetched from, with weights, trust floors, and timeouts.
/// - `output_sections` / `section_budgets` are the **output-section**
///   side: which bundle sections the executor fills and how much budget
///   each gets.
///
/// One plan carries both so a single `plan_hash` covers the whole
/// compile — there is no second plan type and no lossy projection
/// between them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalPlan {
    pub schema_version: u32,
    pub policy_version: String,
    pub intent: ContextIntent,
    pub intent_source: IntentSource,
    /// Role and risk enter the plan so audits can tie adjustments back
    /// to the request.
    pub role: AgentRole,
    pub risk: RiskClass,
    /// Machine-readable snake_case codes explaining intent resolution
    /// and every policy adjustment the planner applied.
    pub reason_codes: Vec<String>,
    /// One entry per known channel, enabled or disabled, in
    /// [`RetrievalChannel::ORDERED`] order.
    pub channel_plans: Vec<ChannelPlan>,
    /// Output-section side: which bundle sections the executor fills,
    /// with per-section item limits and whether the SessionStart
    /// relevance selector governs them.
    pub output_sections: Vec<PlannedChannel>,
    pub section_budgets: SectionBudgets,
    /// Query the relevance selector scores governed sections against.
    /// `None` disables relevance governance (every candidate survives
    /// the relevance stage and only budgets apply).
    pub relevance_query: Option<String>,
    pub relevance_k: u32,
    /// Version of the SessionStart relevance policy the executor will
    /// apply; travels in the plan so an audit pins both policies.
    pub relevance_policy_version: String,
    pub filters: ContextFilters,
    pub rerank_policy: RerankPolicy,
    pub trust_policy: TrustPolicy,
    pub freshness_policy: FreshnessPolicy,
    pub token_budget: u32,
    pub abstention_policy: AbstentionPolicy,
    /// SHA-256 over the canonical plan JSON with this field empty.
    pub plan_hash: String,
}

impl RetrievalPlan {
    pub fn enabled_channels(&self) -> Vec<RetrievalChannel> {
        self.channel_plans
            .iter()
            .filter(|c| c.enabled)
            .map(|c| c.channel)
            .collect()
    }
}
