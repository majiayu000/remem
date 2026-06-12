use serde::Serialize;

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
pub struct SearchExplainChannel {
    pub name: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    pub hits: Vec<ChannelHit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelHit {
    pub memory_id: i64,
    pub rank: usize,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct ChannelContribution {
    pub channel: String,
    pub rank: usize,
    pub score: f64,
}
