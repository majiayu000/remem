use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const DEFAULT_SUITE: &str = "remem-code-memory";
pub const DEFAULT_PUBLIC_ROOT: &str = "eval/public";
pub const DEFAULT_SUITE_ROOT: &str = "eval/public/memory/suites";
pub const DEFAULT_REPORT_BENCHMARK_VERSION: &str = "v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryBenchCondition {
    NoMemory,
    OracleEvidence,
    CompleteStoredMemory,
    RetrievedMemory,
    RememDefault,
}

impl MemoryBenchCondition {
    pub const ALL: [Self; 5] = [
        Self::NoMemory,
        Self::OracleEvidence,
        Self::CompleteStoredMemory,
        Self::RetrievedMemory,
        Self::RememDefault,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoMemory => "no_memory",
            Self::OracleEvidence => "oracle_evidence",
            Self::CompleteStoredMemory => "complete_stored_memory",
            Self::RetrievedMemory => "retrieved_memory",
            Self::RememDefault => "remem_default",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "no_memory" => Some(Self::NoMemory),
            "oracle_evidence" => Some(Self::OracleEvidence),
            "complete_stored_memory" => Some(Self::CompleteStoredMemory),
            "retrieved_memory" => Some(Self::RetrievedMemory),
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

fn active_status() -> String {
    "active".to_string()
}

fn project_scope() -> String {
    "project".to_string()
}
