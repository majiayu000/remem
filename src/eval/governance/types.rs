use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub struct GovernanceEvalOptions {
    pub k: usize,
}

impl Default for GovernanceEvalOptions {
    fn default() -> Self {
        Self { k: 5 }
    }
}

#[derive(Debug, Serialize)]
pub struct GovernanceEvalReport {
    pub metadata: GovernanceEvalMetadata,
    pub metrics: GovernanceMetricSummary,
    pub lifecycle_counts: LifecycleCounts,
    pub summary_candidates: CandidateSummary,
    pub owner_checks: Vec<OwnerCheckReport>,
    pub queries: Vec<QueryReport>,
    pub context: ContextReport,
    pub failing_examples: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GovernanceEvalMetadata {
    pub corpus: String,
    pub storage: String,
    pub data_dir: String,
    pub real_db_touched: bool,
    pub project: String,
    pub nested_projects: Vec<String>,
    pub k: usize,
}

#[derive(Debug, Serialize)]
pub struct GovernanceMetricSummary {
    pub owner_routing_accuracy: RateMetric,
    pub evidence_recall_at_k: RateMetric,
    pub active_current_precision: RateMetric,
    pub stale_exclusion_rate: RateMetric,
    pub context_injection_precision: RateMetric,
    pub all_checks_passed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RateMetric {
    pub passed: usize,
    pub total: usize,
    pub rate: f64,
}

impl RateMetric {
    pub fn new(passed: usize, total: usize) -> Self {
        Self {
            passed,
            total,
            rate: if total == 0 {
                0.0
            } else {
                passed as f64 / total as f64
            },
        }
    }

    pub fn is_perfect(&self) -> bool {
        self.total > 0 && self.passed == self.total
    }
}

#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq)]
pub struct LifecycleCounts {
    pub add: usize,
    pub update: usize,
    pub invalidate: usize,
    pub noop: usize,
    pub defer: usize,
    pub conflict: usize,
}

#[derive(Debug, Serialize)]
pub struct CandidateSummary {
    pub total: usize,
    pub pending_review: usize,
    pub auto_promoted: usize,
    pub active_summary_memories: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnerCheckReport {
    pub object_ref: String,
    pub expected_scope: String,
    pub expected_key: String,
    pub expected_target_project: Option<String>,
    pub actual_scope: Option<String>,
    pub actual_key: Option<String>,
    pub actual_target_project: Option<String>,
    pub pass: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryReport {
    pub id: String,
    pub category: String,
    pub query: String,
    pub project: String,
    pub memory_type: Option<String>,
    pub branch: Option<String>,
    pub expected_topic_keys: Vec<String>,
    pub result_topic_keys: Vec<String>,
    pub result_titles: Vec<String>,
    pub matched_expected: usize,
    pub forbidden_hits: Vec<String>,
    pub unexpected_hits: Vec<String>,
    pub pass: bool,
}

#[derive(Debug, Serialize)]
pub struct ContextReport {
    pub expected_topic_keys: Vec<String>,
    pub included_topic_keys: Vec<String>,
    pub included_titles: Vec<String>,
    pub forbidden_titles: Vec<String>,
    pub unexpected_topic_keys: Vec<String>,
    pub unsafe_owner_included: usize,
    pub excluded_owner_titles: Vec<String>,
    pub pass: bool,
}
