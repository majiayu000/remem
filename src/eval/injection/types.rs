use std::fmt::{self, Display};

use serde::Serialize;

pub use crate::eval::governance::RateMetric as InjectionRateMetric;

pub(super) const CORPUS_NAME: &str = "builtin-context-injection-v2";

#[derive(Debug, Clone, Copy, Default)]
pub struct InjectionEvalOptions {
    pub keep_data_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct InjectionEvalReport {
    pub metadata: InjectionEvalMetadata,
    pub metrics: InjectionMetricSummary,
    pub cases: Vec<InjectionCaseReport>,
    pub failing_examples: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct InjectionEvalMetadata {
    pub corpus: String,
    pub boundary: String,
    pub storage: String,
    pub data_dir: String,
    pub data_dir_kept: bool,
    pub real_db_touched: bool,
    pub project: String,
    pub host: String,
    pub branch: String,
    pub output_chars: usize,
    pub memories_loaded: usize,
    pub core_count: usize,
    pub index_count: usize,
    pub lesson_count: usize,
    pub preference_count: usize,
    pub session_count: usize,
    pub workstream_count: usize,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct InjectionMetricSummary {
    pub expected_memory_recall: InjectionRateMetric,
    pub forbidden_memory_exclusion: InjectionRateMetric,
    pub abstention_false_positive_bound: InjectionRateMetric,
    pub stale_anchor_labeling: InjectionRateMetric,
    pub user_prompt_submit_memory_recall: InjectionRateMetric,
    pub user_prompt_submit_abstention_false_positive_bound: InjectionRateMetric,
    pub all_checks_passed: bool,
}

#[derive(Debug, Serialize)]
pub struct InjectionCaseReport {
    pub id: String,
    pub expectation: String,
    pub title: String,
    pub topic_key: String,
    pub matched: bool,
}

impl Display for InjectionEvalReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== remem eval-injection ({}) ===", self.metadata.corpus)?;
        writeln!(f, "boundary: {}", self.metadata.boundary)?;
        writeln!(
            f,
            "storage: {}; real_db_touched={}",
            self.metadata.storage, self.metadata.real_db_touched
        )?;
        writeln!(
            f,
            "expected_memory_recall: {}/{} ({:.1}%)",
            self.metrics.expected_memory_recall.passed,
            self.metrics.expected_memory_recall.total,
            self.metrics.expected_memory_recall.rate * 100.0
        )?;
        writeln!(
            f,
            "forbidden_memory_exclusion: {}/{} ({:.1}%)",
            self.metrics.forbidden_memory_exclusion.passed,
            self.metrics.forbidden_memory_exclusion.total,
            self.metrics.forbidden_memory_exclusion.rate * 100.0
        )?;
        writeln!(
            f,
            "abstention_false_positive_bound: {}/{} ({:.1}%)",
            self.metrics.abstention_false_positive_bound.passed,
            self.metrics.abstention_false_positive_bound.total,
            self.metrics.abstention_false_positive_bound.rate * 100.0
        )?;
        writeln!(
            f,
            "stale_anchor_labeling: {}/{} ({:.1}%)",
            self.metrics.stale_anchor_labeling.passed,
            self.metrics.stale_anchor_labeling.total,
            self.metrics.stale_anchor_labeling.rate * 100.0
        )?;
        writeln!(
            f,
            "user_prompt_submit_memory_recall: {}/{} ({:.1}%)",
            self.metrics.user_prompt_submit_memory_recall.passed,
            self.metrics.user_prompt_submit_memory_recall.total,
            self.metrics.user_prompt_submit_memory_recall.rate * 100.0
        )?;
        writeln!(
            f,
            "user_prompt_submit_abstention_false_positive_bound: {}/{} ({:.1}%)",
            self.metrics
                .user_prompt_submit_abstention_false_positive_bound
                .passed,
            self.metrics
                .user_prompt_submit_abstention_false_positive_bound
                .total,
            self.metrics
                .user_prompt_submit_abstention_false_positive_bound
                .rate
                * 100.0
        )?;
        writeln!(
            f,
            "rendered: memories_loaded={} core={} index={} chars={} truncated={}",
            self.metadata.memories_loaded,
            self.metadata.core_count,
            self.metadata.index_count,
            self.metadata.output_chars,
            self.metadata.truncated
        )?;
        writeln!(f, "all_checks_passed: {}", self.metrics.all_checks_passed)?;
        if self.failing_examples.is_empty() {
            return Ok(());
        }
        writeln!(f, "failures:")?;
        for failure in &self.failing_examples {
            writeln!(f, "- {failure}")?;
        }
        Ok(())
    }
}
