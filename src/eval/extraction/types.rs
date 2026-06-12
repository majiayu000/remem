use std::fmt::{self, Display};

use serde::{Deserialize, Serialize};

pub type ExtractionRateMetric = crate::eval::governance::RateMetric;

pub const DEFAULT_CORPUS_PATH: &str = "eval/extraction/corpus.json";
pub const DEFAULT_BASELINE_PATH: &str = "eval/extraction/baseline.json";

#[derive(Debug, Clone)]
pub struct ExtractionEvalOptions {
    pub corpus_path: String,
}

impl Default for ExtractionEvalOptions {
    fn default() -> Self {
        Self {
            corpus_path: DEFAULT_CORPUS_PATH.to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ExtractionCorpus {
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub cases: Vec<ExtractionCase>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ExtractionCase {
    pub id: String,
    #[serde(default)]
    pub transcript: Vec<TranscriptEvent>,
    pub observation_output: String,
    pub candidate_output: String,
    #[serde(default)]
    pub expected_observations: Vec<ObservationExpectation>,
    #[serde(default)]
    pub forbidden_observations: Vec<ObservationExpectation>,
    #[serde(default)]
    pub expected_candidates: Vec<CandidateExpectation>,
    #[serde(default)]
    pub forbidden_candidates: Vec<CandidateExpectation>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TranscriptEvent {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub token_estimate: Option<i64>,
    #[serde(default)]
    pub created_at_epoch: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ObservationExpectation {
    pub id: String,
    #[serde(default)]
    pub observation_type: Option<String>,
    #[serde(default)]
    pub text_contains: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CandidateExpectation {
    pub id: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub topic_key: Option<String>,
    #[serde(default)]
    pub risk_class: Option<String>,
    #[serde(default)]
    pub text_contains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExtractionEvalReport {
    pub metadata: ExtractionEvalMetadata,
    pub metrics: ExtractionMetricSummary,
    pub cases: Vec<ExtractionCaseReport>,
    pub failing_examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExtractionEvalMetadata {
    pub corpus: String,
    pub corpus_version: String,
    pub description: String,
    pub cases: usize,
    pub transcript_events: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExtractionMetricSummary {
    pub observation_precision: ExtractionRateMetric,
    pub observation_recall: ExtractionRateMetric,
    pub candidate_precision: ExtractionRateMetric,
    pub candidate_recall: ExtractionRateMetric,
    pub forbidden_observation_exclusion: ExtractionRateMetric,
    pub forbidden_candidate_exclusion: ExtractionRateMetric,
    pub over_saved_predictions: usize,
    pub total_predictions: usize,
    pub over_save_penalty: f64,
    pub all_checks_passed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExtractionCaseReport {
    pub id: String,
    pub transcript_events: usize,
    pub observation_request_sha256: String,
    pub candidate_request_sha256: String,
    pub predicted_observations: Vec<ObservationPrediction>,
    pub predicted_candidates: Vec<CandidatePrediction>,
    pub missing_expected_observations: Vec<String>,
    pub unexpected_observations: Vec<usize>,
    pub forbidden_observations: Vec<String>,
    pub missing_expected_candidates: Vec<String>,
    pub unexpected_candidates: Vec<usize>,
    pub forbidden_candidates: Vec<String>,
    pub over_saved_predictions: usize,
    pub pass: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ObservationPrediction {
    pub index: usize,
    pub observation_type: String,
    pub text: String,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CandidatePrediction {
    pub index: usize,
    pub scope: String,
    pub memory_type: String,
    pub topic_key: String,
    pub risk_class: String,
    pub text: String,
}

impl Display for ExtractionEvalReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Extraction eval: {} cases from {}",
            self.metadata.cases, self.metadata.corpus
        )?;
        writeln!(
            f,
            "Observations: precision {} recall {} forbidden exclusion {}",
            format_rate(&self.metrics.observation_precision),
            format_rate(&self.metrics.observation_recall),
            format_rate(&self.metrics.forbidden_observation_exclusion)
        )?;
        writeln!(
            f,
            "Candidates: precision {} recall {} forbidden exclusion {}",
            format_rate(&self.metrics.candidate_precision),
            format_rate(&self.metrics.candidate_recall),
            format_rate(&self.metrics.forbidden_candidate_exclusion)
        )?;
        writeln!(
            f,
            "Over-save penalty: {:.4} ({}/{})",
            self.metrics.over_save_penalty,
            self.metrics.over_saved_predictions,
            self.metrics.total_predictions
        )?;
        writeln!(f, "All checks passed: {}", self.metrics.all_checks_passed)?;
        if !self.failing_examples.is_empty() {
            writeln!(f, "Failures:")?;
            for failure in &self.failing_examples {
                writeln!(f, "- {failure}")?;
            }
        }
        Ok(())
    }
}

fn format_rate(metric: &ExtractionRateMetric) -> String {
    format!("{}/{} ({:.4})", metric.passed, metric.total, metric.rate)
}
