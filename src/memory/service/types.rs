#[derive(Debug, Clone, Default)]
pub struct SearchRequest {
    pub query: Option<String>,
    pub project: Option<String>,
    pub memory_type: Option<String>,
    pub limit: i64,
    pub offset: i64,
    pub include_stale: bool,
    pub include_suppressed: bool,
    pub branch: Option<String>,
    pub multi_hop: bool,
    pub explain: bool,
}

#[derive(Debug, Clone)]
pub struct SearchRoutingPolicy {
    pub plan_hash: String,
    pub policy_version: String,
    pub rerank_enabled: bool,
    pub rerank_candidate_pool: u32,
    pub rerank_output_k: u32,
    pub use_multi_hop: bool,
    pub raw_fallback_enabled: bool,
    pub weights: SearchRoutingWeights,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchRoutingWeights {
    pub fts: f64,
    pub vector: f64,
    pub entity: f64,
    pub graph: f64,
    pub temporal: f64,
    pub fact: f64,
    pub like_fallback: f64,
    pub usage: f64,
}

impl SearchRoutingPolicy {
    pub fn from_retrieval_plan(plan: &crate::retrieval_router::RetrievalPlan) -> Self {
        let weight = |channel| channel_weight(plan, channel);
        let temporal = weight(crate::retrieval_router::RetrievalChannel::Temporal);
        Self {
            plan_hash: plan.plan_hash.clone(),
            policy_version: plan.policy_version.clone(),
            rerank_enabled: plan.rerank_policy.enabled,
            rerank_candidate_pool: plan.rerank_policy.candidate_pool,
            rerank_output_k: plan.rerank_policy.output_k,
            use_multi_hop: channel_enabled(
                plan,
                crate::retrieval_router::RetrievalChannel::GraphExpansion,
            ),
            raw_fallback_enabled: plan.abstention_policy.mode
                != crate::retrieval_router::AbstentionMode::OnLowEvidence,
            weights: SearchRoutingWeights {
                fts: weight(crate::retrieval_router::RetrievalChannel::CanonicalFts),
                vector: weight(crate::retrieval_router::RetrievalChannel::CanonicalVector),
                entity: weight(crate::retrieval_router::RetrievalChannel::EntityGraph),
                graph: weight(crate::retrieval_router::RetrievalChannel::GraphExpansion),
                temporal,
                // Structured fact lookup is the production implementation of
                // the router's temporal/fact evidence lane.
                fact: temporal,
                // LIKE is retained only as the canonical FTS degradation path.
                like_fallback: if channel_enabled(
                    plan,
                    crate::retrieval_router::RetrievalChannel::CanonicalFts,
                ) {
                    0.25
                } else {
                    0.0
                },
                // Usage ranking is not a GH-934 retrieval-router channel; keep
                // routed execution bounded to channels represented in the plan.
                usage: 0.0,
            },
        }
    }
}

fn channel_enabled(
    plan: &crate::retrieval_router::RetrievalPlan,
    channel: crate::retrieval_router::RetrievalChannel,
) -> bool {
    plan.channel_plans
        .iter()
        .any(|plan| plan.channel == channel && plan.enabled)
}

fn channel_weight(
    plan: &crate::retrieval_router::RetrievalPlan,
    channel: crate::retrieval_router::RetrievalChannel,
) -> f64 {
    plan.channel_plans
        .iter()
        .find(|plan| plan.channel == channel && plan.enabled)
        .map(|plan| plan.weight)
        .unwrap_or(0.0)
}

/// Canonical default for `include_stale` across every adapter (MCP, REST, CLI).
///
/// Default search returns only current curated memories. Callers that need
/// stale or archived history must opt in explicitly.
pub fn default_include_stale() -> bool {
    false
}

/// Canonical default for `include_suppressed` across every adapter.
pub fn default_include_suppressed() -> bool {
    false
}

#[derive(Debug, Clone)]
pub struct MultiHopMeta {
    pub hops: u8,
    pub entities_discovered: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SearchResultSet {
    pub memories: Vec<crate::memory::Memory>,
    pub multi_hop: Option<MultiHopMeta>,
    pub has_more: bool,
    pub explain: Option<crate::retrieval::search::SearchExplain>,
    /// Raw archive hits attached as fallback when curated memories are sparse.
    pub raw_hits: Vec<crate::memory::raw_archive::RawMessage>,
    /// Error from the raw archive fallback path. Curated results remain usable.
    pub raw_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SearchResultSetWithExplainDetails {
    pub result: SearchResultSet,
    pub explain_details: Option<crate::retrieval::search::SearchExplainDetails>,
}

#[derive(Debug, Clone, Default)]
pub struct SaveMemoryRequest {
    pub text: String,
    pub title: Option<String>,
    pub project: Option<String>,
    pub session_id: Option<String>,
    pub host: Option<String>,
    pub topic_key: Option<String>,
    pub memory_type: Option<String>,
    pub files: Option<Vec<String>>,
    pub scope: Option<String>,
    pub created_at_epoch: Option<i64>,
    pub branch: Option<String>,
    pub local_path: Option<String>,
    pub local_copy_enabled: Option<bool>,
    pub claim_enabled: Option<bool>,
    pub claim_source: Option<String>,
    pub acknowledge_pattern: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LocalCopyResult {
    pub status: String,
    pub path: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SaveMemoryNextStep {
    pub tool: String,
    pub ids: Vec<i64>,
    pub source: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct SaveMemoryResult {
    pub id: i64,
    pub status: String,
    pub memory_type: String,
    pub project: String,
    pub scope: String,
    pub topic_key: Option<String>,
    pub branch: Option<String>,
    pub operation: String,
    pub created_at_epoch: i64,
    pub reference_time_epoch: i64,
    pub updated_at_epoch: i64,
    /// Compatibility alias: true when the request supplied `topic_key`.
    /// It does not mean the durable row was updated; use `operation` for that.
    pub upserted: bool,
    pub local_copy: LocalCopyResult,
    pub local_status: String,
    pub local_path: Option<String>,
    pub claim_status: String,
    pub claim_id: Option<i64>,
    pub claim_error: Option<String>,
    pub next_step: SaveMemoryNextStep,
}
