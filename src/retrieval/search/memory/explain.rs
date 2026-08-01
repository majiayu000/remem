use serde::{ser::SerializeStruct, Serialize, Serializer};

#[derive(Debug, Clone, Serialize)]
pub struct SearchExplain {
    pub query: String,
    pub project: Option<String>,
    pub memory_type: Option<String>,
    pub branch: Option<String>,
    pub include_stale: bool,
    pub limit: i64,
    pub offset: i64,
    pub fetch_limit: i64,
    pub expanded_terms: Vec<String>,
    pub core_terms: Vec<String>,
    pub claim_terms: Vec<String>,
    pub fts_query: Option<String>,
    pub temporal_range: Option<(i64, i64)>,
    pub temporal_field: Option<String>,
    pub rrf_k: f64,
    pub min_evidence_confidence: f64,
    pub filtered_result_count: usize,
    pub timings: Vec<crate::perf::PhaseTiming>,
    /// Rerank is a dedicated post-fusion stage, not a recall channel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerank: Option<crate::retrieval::rerank::RerankExplain>,
    pub channels: Vec<SearchExplainChannel>,
    pub results: Vec<SearchExplainResult>,
    pub has_more: bool,
    pub raw_fallback_count: usize,
}

impl SearchExplain {
    pub fn retain_result_ids(&mut self, result_ids: &[i64], has_more: bool, visible_limit: i64) {
        self.has_more = has_more;
        self.limit = visible_limit;
        self.results
            .retain(|result| result_ids.contains(&result.memory_id));
        for (index, result) in self.results.iter_mut().enumerate() {
            result.final_rank = index + 1;
        }
    }

    pub fn set_raw_fallback_count(&mut self, count: usize) {
        self.raw_fallback_count = count;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchExplainDetails {
    #[serde(flatten)]
    pub explain: SearchExplain,
    pub contribution_breakdowns: Vec<SearchExplainResultBreakdown>,
}

impl SearchExplainDetails {
    pub fn retain_result_ids(&mut self, result_ids: &[i64], has_more: bool, visible_limit: i64) {
        self.explain
            .retain_result_ids(result_ids, has_more, visible_limit);
        self.contribution_breakdowns
            .retain(|result| result_ids.contains(&result.memory_id));
    }

    pub fn set_raw_fallback_count(&mut self, count: usize) {
        self.explain.set_raw_fallback_count(count);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchExplainChannel {
    pub name: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates_scanned: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<crate::retrieval::embedding::EmbeddingExecutionMetadata>,
    pub hits: Vec<ChannelHit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelHit {
    pub memory_id: i64,
    pub rank: usize,
}

#[derive(Debug, Clone)]
pub struct SearchExplainResult {
    pub memory_id: i64,
    pub final_rank: usize,
    pub final_score: f64,
    pub evidence_confidence: f64,
    pub project: String,
    pub scope: String,
    pub visibility: String,
    pub staleness: crate::memory::MemoryStalenessLabel,
    pub contributions: Vec<ChannelContribution>,
}

impl SearchExplainResult {
    /// Sum of the per-channel RRF contributions before post-fusion policies.
    pub fn fusion_score(&self) -> f64 {
        self.contributions
            .iter()
            .map(|contribution| contribution.score)
            .sum()
    }

    /// Multiplier applied after fusion, such as source-anchor demotion.
    pub fn post_fusion_score_factor(&self) -> Option<f64> {
        let fusion_score = self.fusion_score();
        (fusion_score.is_finite() && fusion_score > 0.0)
            .then_some(self.final_score / fusion_score)
            .filter(|factor| factor.is_finite() && *factor >= 0.0)
    }
}

impl Serialize for SearchExplainResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("SearchExplainResult", 11)?;
        state.serialize_field("memory_id", &self.memory_id)?;
        state.serialize_field("final_rank", &self.final_rank)?;
        state.serialize_field("final_score", &self.final_score)?;
        state.serialize_field("evidence_confidence", &self.evidence_confidence)?;
        state.serialize_field("project", &self.project)?;
        state.serialize_field("scope", &self.scope)?;
        state.serialize_field("visibility", &self.visibility)?;
        state.serialize_field("staleness", &self.staleness)?;
        state.serialize_field("contributions", &self.contributions)?;
        state.serialize_field("fusion_score", &self.fusion_score())?;
        state.serialize_field("post_fusion_score_factor", &self.post_fusion_score_factor())?;
        state.end()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelContribution {
    pub channel: String,
    pub rank: usize,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchExplainResultBreakdown {
    pub memory_id: i64,
    pub contributions: Vec<ChannelContributionBreakdown>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelContributionBreakdown {
    pub channel: String,
    pub rank: usize,
    pub weight: f64,
    pub reciprocal_rank: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_signal: Option<f64>,
    pub total_score: f64,
}
