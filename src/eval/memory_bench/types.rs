use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const DEFAULT_SUITE: &str = "remem-code-memory";
pub const ADVERSARIAL_POLICY_SUITE: &str = "adversarial-policy";
pub const SUPPORTED_SUITES: [&str; 2] = [DEFAULT_SUITE, ADVERSARIAL_POLICY_SUITE];
pub const DEFAULT_PUBLIC_ROOT: &str = "eval/public";
pub const DEFAULT_SUITE_ROOT: &str = "eval/public/memory/suites";
pub const DEFAULT_REPORT_BENCHMARK_VERSION: &str = "v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryBenchCondition {
    NoMemory,
    TruncatedFullContext,
    OracleEvidence,
    CompleteStoredMemory,
    RetrievedMemory,
    Bm25Baseline,
    VectorBaseline,
    HybridRagBaseline,
    SummaryBaseline,
    RememDefault,
}

impl MemoryBenchCondition {
    pub const ALL: [Self; 10] = [
        Self::NoMemory,
        Self::TruncatedFullContext,
        Self::OracleEvidence,
        Self::CompleteStoredMemory,
        Self::RetrievedMemory,
        Self::Bm25Baseline,
        Self::VectorBaseline,
        Self::HybridRagBaseline,
        Self::SummaryBaseline,
        Self::RememDefault,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoMemory => "no_memory",
            Self::TruncatedFullContext => "truncated_full_context",
            Self::OracleEvidence => "oracle_evidence",
            Self::CompleteStoredMemory => "complete_stored_memory",
            Self::RetrievedMemory => "retrieved_memory",
            Self::Bm25Baseline => "bm25_baseline",
            Self::VectorBaseline => "vector_baseline",
            Self::HybridRagBaseline => "hybrid_rag_baseline",
            Self::SummaryBaseline => "summary_baseline",
            Self::RememDefault => "remem_default",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "no_memory" => Some(Self::NoMemory),
            "truncated_full_context" => Some(Self::TruncatedFullContext),
            "oracle_evidence" => Some(Self::OracleEvidence),
            "complete_stored_memory" => Some(Self::CompleteStoredMemory),
            "retrieved_memory" => Some(Self::RetrievedMemory),
            "bm25_baseline" => Some(Self::Bm25Baseline),
            "vector_baseline" => Some(Self::VectorBaseline),
            "hybrid_rag_baseline" => Some(Self::HybridRagBaseline),
            "summary_baseline" => Some(Self::SummaryBaseline),
            "remem_default" => Some(Self::RememDefault),
            _ => None,
        }
    }
}

impl fmt::Display for MemoryBenchCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryBenchSuiteFixture {
    pub schema_version: u32,
    pub suite: String,
    pub version: String,
    pub fixture_revision: String,
    pub benchmark_id: String,
    pub tasks: Vec<MemoryBenchTask>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryBenchTask {
    pub id: String,
    pub category: String,
    pub reference_time_epoch: i64,
    pub prompt: String,
    pub query: String,
    pub expected_answer: String,
    #[serde(default)]
    pub abstention_allowed: bool,
    #[serde(default)]
    pub gold_supporting_event_ids: Vec<String>,
    #[serde(default)]
    pub forbidden_event_ids: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<MemoryBenchEvidence>,
    #[serde(default)]
    pub policy: Option<MemoryBenchPolicyExpectation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryBenchEvidence {
    pub event_id: String,
    pub title: String,
    pub content: String,
    pub memory_type: String,
    #[serde(default = "active_status")]
    pub status: String,
    #[serde(default = "project_scope")]
    pub scope: String,
    #[serde(default)]
    pub topic_key: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub source_anchor: String,
    #[serde(default)]
    pub created_at_epoch: Option<i64>,
    #[serde(default = "retention_allowed")]
    pub retention_allowed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryBenchPolicyExpectation {
    #[serde(default)]
    pub non_retention_required: bool,
    #[serde(default)]
    pub explicit_approval: bool,
    #[serde(default)]
    pub expected_active_claims: u32,
    #[serde(default)]
    pub expected_candidates: u32,
    #[serde(default)]
    pub expected_summary_inputs: u32,
    #[serde(default)]
    pub sensitive_or_restricted: bool,
    #[serde(default)]
    pub suppression_expected: bool,
    #[serde(default)]
    pub expected_policy_abstention: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryBenchPolicySummary {
    pub non_retention_cases: usize,
    pub non_retention_leak_rate: f64,
    pub false_block_cases: usize,
    pub false_block_rate: f64,
    pub suppression_cases: usize,
    pub suppression_obeyed_rate: f64,
    pub sensitive_restricted_cases: usize,
    pub sensitive_restricted_default_exclusion_rate: f64,
    pub policy_abstention_cases: usize,
    pub policy_abstention_accuracy: f64,
    pub policy_failure_rate: f64,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryBenchPolicyOutcome {
    pub active_claim_count: u32,
    pub candidate_count: u32,
    pub summary_input_count: u32,
    pub non_retention_applicable: bool,
    pub non_retention_leaked: bool,
    pub false_block_applicable: bool,
    pub false_blocked: bool,
    pub suppression_applicable: bool,
    pub suppression_obeyed: bool,
    pub sensitive_restricted_applicable: bool,
    pub sensitive_restricted_default_excluded: bool,
    pub policy_abstention_applicable: bool,
    pub policy_abstention_correct: bool,
    pub policy_failure_count: usize,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MemoryBenchMetricSummary {
    pub tasks: usize,
    pub support_coverage: f64,
    pub answer_score: f64,
    pub citation_recall: f64,
    pub citation_precision: f64,
    pub staleness_accuracy: f64,
    pub abstention_accuracy: f64,
    pub forbidden_evidence_rate: f64,
}

#[derive(Debug, Clone)]
pub struct MemoryBenchRunOutcome {
    pub condition: MemoryBenchCondition,
    pub task_id: String,
    pub category: String,
    pub run_index: u32,
    pub retrieved_memory_ids: Vec<i64>,
    pub retrieved_event_ids: Vec<String>,
    pub cited_memory_ids: Vec<i64>,
    pub cited_event_ids: Vec<String>,
    pub missing_event_ids: Vec<String>,
    pub answer_text: String,
    pub abstained: bool,
    pub support_coverage: f64,
    pub answer_score: f64,
    pub citation_recall: f64,
    pub citation_precision: f64,
    pub staleness_accuracy: f64,
    pub abstention_accuracy: f64,
    pub forbidden_evidence_count: usize,
    pub reader_input: String,
    pub retrieved_evidence_json: serde_json::Value,
    pub diagnosis_notes: Vec<String>,
    pub policy: MemoryBenchPolicyOutcome,
    pub diagnosis: MemoryBenchDiagnosisOutcome,
    pub performance: MemoryBenchPerformanceMetrics,
}

impl MemoryBenchRunOutcome {
    pub fn summary(&self) -> MemoryBenchMetricSummary {
        MemoryBenchMetricSummary {
            tasks: 1,
            support_coverage: self.support_coverage,
            answer_score: self.answer_score,
            citation_recall: self.citation_recall,
            citation_precision: self.citation_precision,
            staleness_accuracy: self.staleness_accuracy,
            abstention_accuracy: self.abstention_accuracy,
            forbidden_evidence_rate: if self.forbidden_evidence_count == 0 {
                0.0
            } else {
                1.0
            },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryBenchDiagnosisOutcome {
    pub write_side_gap: bool,
    pub retrieval_side_gap: bool,
    pub reader_gap: bool,
    pub policy_abstention: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MemoryBenchPerformanceMetrics {
    pub ingest_tokens: u64,
    pub query_tokens: u64,
    pub reader_tokens: u64,
    pub retrieval_latency_ms: u64,
    pub end_to_end_latency_ms: u64,
    pub rows_written: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MemoryBenchPerformanceSummary {
    pub tasks: usize,
    pub ingest_tokens_mean: f64,
    pub query_tokens_mean: f64,
    pub reader_tokens_mean: f64,
    pub retrieval_latency_p50_ms: f64,
    pub retrieval_latency_p95_ms: f64,
    pub end_to_end_latency_p50_ms: f64,
    pub end_to_end_latency_p95_ms: f64,
    pub rows_written_mean: f64,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MemoryBenchFailureDecomposition {
    pub runs: usize,
    pub write_side_evidence_loss: usize,
    pub retrieval_miss: usize,
    pub reader_failure: usize,
    pub policy_abstention: usize,
    pub clean_runs: usize,
}

pub fn summarize_metrics<'a>(
    runs: impl IntoIterator<Item = &'a MemoryBenchRunOutcome>,
) -> MemoryBenchMetricSummary {
    let mut summary = MemoryBenchMetricSummary::default();
    for run in runs {
        let item = run.summary();
        summary.tasks += 1;
        summary.support_coverage += item.support_coverage;
        summary.answer_score += item.answer_score;
        summary.citation_recall += item.citation_recall;
        summary.citation_precision += item.citation_precision;
        summary.staleness_accuracy += item.staleness_accuracy;
        summary.abstention_accuracy += item.abstention_accuracy;
        summary.forbidden_evidence_rate += item.forbidden_evidence_rate;
    }
    if summary.tasks > 0 {
        let divisor = summary.tasks as f64;
        summary.support_coverage /= divisor;
        summary.answer_score /= divisor;
        summary.citation_recall /= divisor;
        summary.citation_precision /= divisor;
        summary.staleness_accuracy /= divisor;
        summary.abstention_accuracy /= divisor;
        summary.forbidden_evidence_rate /= divisor;
    }
    summary
}

pub fn summarize_by_category(
    outcomes: &[MemoryBenchRunOutcome],
) -> BTreeMap<String, MemoryBenchMetricSummary> {
    let mut grouped: BTreeMap<String, Vec<&MemoryBenchRunOutcome>> = BTreeMap::new();
    for outcome in outcomes {
        grouped
            .entry(outcome.category.clone())
            .or_default()
            .push(outcome);
    }
    grouped
        .into_iter()
        .map(|(category, runs)| (category, summarize_metrics(runs)))
        .collect()
}

pub fn summarize_policy(outcomes: &[MemoryBenchRunOutcome]) -> MemoryBenchPolicySummary {
    let non_retention = outcomes
        .iter()
        .filter(|outcome| outcome.policy.non_retention_applicable)
        .collect::<Vec<_>>();
    let false_block = outcomes
        .iter()
        .filter(|outcome| outcome.policy.false_block_applicable)
        .collect::<Vec<_>>();
    let suppression = outcomes
        .iter()
        .filter(|outcome| outcome.policy.suppression_applicable)
        .collect::<Vec<_>>();
    let sensitive = outcomes
        .iter()
        .filter(|outcome| outcome.policy.sensitive_restricted_applicable)
        .collect::<Vec<_>>();
    let abstention = outcomes
        .iter()
        .filter(|outcome| outcome.policy.policy_abstention_applicable)
        .collect::<Vec<_>>();
    let policy_failure_count = outcomes
        .iter()
        .filter(|outcome| outcome.policy.policy_failure_count > 0)
        .count();

    MemoryBenchPolicySummary {
        non_retention_cases: non_retention.len(),
        non_retention_leak_rate: ratio_count(
            non_retention
                .iter()
                .filter(|outcome| outcome.policy.non_retention_leaked)
                .count(),
            non_retention.len(),
        ),
        false_block_cases: false_block.len(),
        false_block_rate: ratio_count(
            false_block
                .iter()
                .filter(|outcome| outcome.policy.false_blocked)
                .count(),
            false_block.len(),
        ),
        suppression_cases: suppression.len(),
        suppression_obeyed_rate: ratio_count(
            suppression
                .iter()
                .filter(|outcome| outcome.policy.suppression_obeyed)
                .count(),
            suppression.len(),
        ),
        sensitive_restricted_cases: sensitive.len(),
        sensitive_restricted_default_exclusion_rate: ratio_count(
            sensitive
                .iter()
                .filter(|outcome| outcome.policy.sensitive_restricted_default_excluded)
                .count(),
            sensitive.len(),
        ),
        policy_abstention_cases: abstention.len(),
        policy_abstention_accuracy: ratio_count(
            abstention
                .iter()
                .filter(|outcome| outcome.policy.policy_abstention_correct)
                .count(),
            abstention.len(),
        ),
        policy_failure_rate: ratio_count(policy_failure_count, outcomes.len()),
    }
}

fn ratio_count(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn active_status() -> String {
    "active".to_string()
}

fn project_scope() -> String {
    "project".to_string()
}

fn retention_allowed() -> bool {
    true
}
