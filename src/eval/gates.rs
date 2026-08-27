//! Eval gate comparison: baseline vs current metrics with max-drop,
//! max-increase, and strictly-positive minimum thresholds.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DEFAULT_BASELINE_PATH: &str = "eval/gates/baseline.json";
pub const DEFAULT_THRESHOLDS_PATH: &str = "eval/gates/thresholds.json";
pub const DEFAULT_GOLDEN_DATASET_PATH: &str = "eval/golden.json";

#[derive(Debug, Clone)]
pub struct EvalGateOptions {
    pub baseline_path: String,
    pub thresholds_path: String,
    pub golden_dataset_path: String,
    pub simulate_golden_regression: bool,
    pub simulate_capacity_regression: bool,
}

impl Default for EvalGateOptions {
    fn default() -> Self {
        Self {
            baseline_path: DEFAULT_BASELINE_PATH.to_string(),
            thresholds_path: DEFAULT_THRESHOLDS_PATH.to_string(),
            golden_dataset_path: DEFAULT_GOLDEN_DATASET_PATH.to_string(),
            simulate_golden_regression: false,
            simulate_capacity_regression: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvalGateBaseline {
    pub version: String,
    pub metrics: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvalGateThresholds {
    pub version: String,
    #[serde(default)]
    pub default_max_drop: f64,
    #[serde(default)]
    pub metrics: BTreeMap<String, EvalGateThreshold>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvalGateThreshold {
    #[serde(default)]
    pub max_drop: f64,
    #[serde(default)]
    pub max_increase: Option<f64>,
    /// Strictly-positive machine minimum: the current value must be greater
    /// than this floor regardless of the baseline (GH-850 paraphrase gate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_value: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalGateReport {
    pub version: String,
    pub baseline_version: String,
    pub thresholds_version: String,
    pub summary: EvalGateSummary,
    pub deltas: Vec<EvalGateDelta>,
    pub failures: Vec<String>,
    pub source_reports: EvalSourceReports,
    pub source_artifacts: BTreeMap<String, EvalSourceArtifact>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalSourceArtifact {
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalGateSummary {
    pub metrics_checked: usize,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalGateDelta {
    pub metric: String,
    pub baseline: f64,
    pub current: f64,
    pub delta: f64,
    pub max_drop: f64,
    pub status: EvalGateStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalGateStatus {
    Pass,
    Fail,
    MissingCurrent,
    MissingBaseline,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalSourceReports {
    pub current_memory_contracts: serde_json::Value,
    pub capacity: serde_json::Value,
    pub golden: serde_json::Value,
    pub injection: serde_json::Value,
    pub extraction: serde_json::Value,
}

pub(crate) struct EvalGateExecution {
    pub(crate) legacy_report: EvalGateReport,
    pub(crate) report_json: String,
    pub(crate) ship_summary: String,
    pub(crate) command_passed: bool,
}

pub(crate) fn run_eval_gates_with_ship_evidence(
    options: EvalGateOptions,
) -> Result<EvalGateExecution> {
    let report = run_eval_gates(options)?;
    let ship_options = crate::eval::ship_matrix::ShipMatrixOptions {
        baseline_path: report
            .source_artifacts
            .keys()
            .find(|path| path.ends_with("baseline.json"))
            .cloned()
            .unwrap_or_else(|| DEFAULT_BASELINE_PATH.to_string()),
        thresholds_path: report
            .source_artifacts
            .keys()
            .find(|path| path.ends_with("thresholds.json"))
            .cloned()
            .unwrap_or_else(|| DEFAULT_THRESHOLDS_PATH.to_string()),
        golden_dataset_path: report
            .source_artifacts
            .keys()
            .find(|path| path.ends_with("golden.json"))
            .cloned()
            .unwrap_or_else(|| DEFAULT_GOLDEN_DATASET_PATH.to_string()),
        input_artifact_sha256: report
            .source_artifacts
            .iter()
            .map(|(path, artifact)| (path.clone(), artifact.sha256.clone()))
            .collect(),
        ..Default::default()
    };
    let capacity_applicable = report
        .source_reports
        .capacity
        .get("skipped")
        .and_then(serde_json::Value::as_bool)
        != Some(true);
    let ship_evidence = crate::eval::ship_matrix::build_ship_evidence(
        &report.deltas,
        capacity_applicable,
        report.summary.passed,
        ship_options,
    );
    let command_passed = ship_evidence.ship_matrix.summary.command_passed;
    let summary = &ship_evidence.ship_matrix.summary;
    let ship_summary = format_ship_summary(summary);
    let mut report_value = serde_json::to_value(&report)?;
    let report_object = report_value
        .as_object_mut()
        .context("eval-gates report must serialize as a JSON object")?;
    report_object.insert(
        "ship_matrix".to_string(),
        serde_json::to_value(&ship_evidence.ship_matrix)?,
    );
    report_object.insert(
        "outcome_scorecard".to_string(),
        serde_json::to_value(&ship_evidence.outcome_scorecard)?,
    );
    report_object
        .get_mut("summary")
        .and_then(serde_json::Value::as_object_mut)
        .context("eval-gates report summary must serialize as a JSON object")?
        .insert("passed".to_string(), serde_json::json!(command_passed));
    Ok(EvalGateExecution {
        legacy_report: report,
        report_json: serde_json::to_string_pretty(&report_value)?,
        ship_summary,
        command_passed,
    })
}

fn format_ship_summary(summary: &crate::eval::ship_matrix::ShipMatrixSummary) -> String {
    format!(
        "ship_matrix command_passed={} merge_ready={} release_ready={} default_on_ready={} cross_host_claim_ready={} coding_outcome_claim_ready={} public_claim_ready={}",
        summary.command_passed,
        summary.merge_ready,
        summary.release_ready,
        summary.default_on_ready,
        summary.cross_host_claim_ready,
        summary.coding_outcome_claim_ready,
        summary.public_claim_ready,
    )
}

pub fn run_eval_gates(options: EvalGateOptions) -> Result<EvalGateReport> {
    let (mut baseline, baseline_artifact) = load_baseline(&options.baseline_path)?;
    let (mut thresholds, thresholds_artifact) = load_thresholds(&options.thresholds_path)?;
    let (golden_dataset, golden_artifact) = load_golden(&options.golden_dataset_path)?;
    let golden = run_golden(&golden_dataset)?;
    let capacity = if golden_dataset.has_fixture_corpus() {
        Some(crate::eval::capacity::run_capacity_eval_for_dataset(
            crate::eval::capacity::CapacityEvalOptions {
                dataset_path: options.golden_dataset_path.clone(),
                seed: 42,
                scales: vec![1, 10],
                k: 5,
            },
            golden_dataset,
        )?)
    } else {
        remove_capacity_gate_metrics(&mut baseline, &mut thresholds);
        None
    };
    let current_memory_contracts =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    let injection = crate::eval::injection::run_sandbox_eval(Default::default())?;
    let extraction = crate::eval::extraction::run_corpus_path(Default::default())?;

    let mut current_metrics = collect_metrics(
        &golden,
        capacity.as_ref(),
        &current_memory_contracts,
        &injection,
        &extraction,
    );
    if options.simulate_golden_regression {
        current_metrics.insert("golden.slice.temporal.hit_at_k".to_string(), 0.0);
    }
    if options.simulate_capacity_regression {
        current_metrics.insert(
            "capacity.degradation.fused.recall_at_k_loss".to_string(),
            1.0,
        );
    }
    let (deltas, failures) = compare_metrics(&baseline, &thresholds, &current_metrics);
    let source_reports = EvalSourceReports {
        current_memory_contracts: serde_json::to_value(&current_memory_contracts)?,
        capacity: match capacity.as_ref() {
            Some(capacity) => serde_json::to_value(capacity)?,
            None => serde_json::json!({
                "skipped": true,
                "reason": "golden dataset has no fixture corpus; capacity eval is not applicable"
            }),
        },
        golden: serde_json::to_value(&golden)?,
        injection: serde_json::to_value(&injection)?,
        extraction: serde_json::to_value(&extraction)?,
    };

    Ok(EvalGateReport {
        version: "2026-06-23".to_string(),
        baseline_version: baseline.version,
        thresholds_version: thresholds.version,
        summary: EvalGateSummary {
            metrics_checked: deltas.len(),
            passed: failures.is_empty(),
        },
        deltas,
        failures,
        source_reports,
        source_artifacts: BTreeMap::from([
            (options.baseline_path, baseline_artifact),
            (options.thresholds_path, thresholds_artifact),
            (options.golden_dataset_path, golden_artifact),
        ]),
    })
}

fn load_baseline(path: &str) -> Result<(EvalGateBaseline, EvalSourceArtifact)> {
    let content = fs::read(path)
        .with_context(|| format!("read eval gate baseline {}", Path::new(path).display()))?;
    let parsed = serde_json::from_slice(&content)
        .with_context(|| format!("parse eval gate baseline {}", Path::new(path).display()))?;
    Ok((parsed, source_artifact(&content)))
}

fn load_thresholds(path: &str) -> Result<(EvalGateThresholds, EvalSourceArtifact)> {
    let content = fs::read(path)
        .with_context(|| format!("read eval gate thresholds {}", Path::new(path).display()))?;
    let parsed = serde_json::from_slice(&content)
        .with_context(|| format!("parse eval gate thresholds {}", Path::new(path).display()))?;
    Ok((parsed, source_artifact(&content)))
}

fn load_golden(path: &str) -> Result<(crate::eval::golden::GoldenDataset, EvalSourceArtifact)> {
    let content = fs::read(path)
        .with_context(|| format!("read golden eval dataset {}", Path::new(path).display()))?;
    let parsed = serde_json::from_slice(&content)
        .with_context(|| format!("parse golden eval dataset {}", Path::new(path).display()))?;
    Ok((parsed, source_artifact(&content)))
}

fn source_artifact(bytes: &[u8]) -> EvalSourceArtifact {
    EvalSourceArtifact {
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

fn run_golden(
    dataset: &crate::eval::golden::GoldenDataset,
) -> Result<crate::eval::golden::GoldenEvalReport> {
    if dataset.has_fixture_corpus() {
        crate::eval::golden::evaluate_dataset_with_fixture_corpus(dataset, 5)
    } else {
        let conn = crate::db::open_db()?;
        crate::eval::golden::evaluate_dataset(&conn, dataset, 5)
    }
}

fn remove_capacity_gate_metrics(
    baseline: &mut EvalGateBaseline,
    thresholds: &mut EvalGateThresholds,
) {
    baseline
        .metrics
        .retain(|metric, _| !metric.starts_with("capacity."));
    thresholds
        .metrics
        .retain(|metric, _| !metric.starts_with("capacity."));
}

fn collect_metrics(
    golden: &crate::eval::golden::GoldenEvalReport,
    capacity: Option<&crate::eval::capacity::CapacityEvalReport>,
    current_memory_contracts: &crate::eval::current_memory_contracts::CurrentMemoryContractEvalReport,
    injection: &crate::eval::injection::InjectionEvalReport,
    extraction: &crate::eval::extraction::ExtractionEvalReport,
) -> BTreeMap<String, f64> {
    let mut metrics = BTreeMap::new();
    metrics.insert(
        "golden.total_queries".to_string(),
        golden.total_queries as f64,
    );
    metrics.insert(
        "golden.scored_queries".to_string(),
        golden.scored_queries as f64,
    );
    if let Some(overall) = golden.overall.as_ref() {
        insert_golden_metrics(&mut metrics, "golden.overall", overall);
    }
    for (slice, evaluation) in &golden.by_slice {
        let prefix = format!("golden.slice.{slice}");
        if let Some(slice_metrics) = evaluation.metrics.as_ref() {
            insert_golden_metrics(&mut metrics, &prefix, slice_metrics);
        }
        if evaluation.abstention_queries > 0 {
            metrics.insert(
                format!("{prefix}.abstention_pass_rate"),
                evaluation.abstention_passed as f64 / evaluation.abstention_queries as f64,
            );
        }
    }
    if let Some(capacity) = capacity {
        insert_capacity_metrics(&mut metrics, capacity);
    }
    metrics.insert(
        "current_memory_contracts.current_state.current".to_string(),
        current_memory_contracts.metrics.current_state.current.rate,
    );
    metrics.insert(
        "current_memory_contracts.current_state.no_current".to_string(),
        current_memory_contracts
            .metrics
            .current_state
            .no_current
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.current_state.unresolved_conflict".to_string(),
        current_memory_contracts
            .metrics
            .current_state
            .unresolved_conflict
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.current_state.ambiguous".to_string(),
        current_memory_contracts
            .metrics
            .current_state
            .ambiguous
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.temporal.invalidated_fact_exclusion".to_string(),
        current_memory_contracts
            .metrics
            .temporal
            .invalidated_fact_exclusion
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.temporal.expired_fact_exclusion".to_string(),
        current_memory_contracts
            .metrics
            .temporal
            .expired_fact_exclusion
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.temporal.as_of_fact_retrieval".to_string(),
        current_memory_contracts
            .metrics
            .temporal
            .as_of_fact_retrieval
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.staleness.tracked".to_string(),
        current_memory_contracts.metrics.staleness.tracked.rate,
    );
    metrics.insert(
        "current_memory_contracts.staleness.untracked".to_string(),
        current_memory_contracts.metrics.staleness.untracked.rate,
    );
    metrics.insert(
        "current_memory_contracts.staleness.history_tracked".to_string(),
        current_memory_contracts
            .metrics
            .staleness
            .history_tracked
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.staleness.verify_before_trust".to_string(),
        current_memory_contracts
            .metrics
            .staleness
            .verify_before_trust
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.staleness.error".to_string(),
        current_memory_contracts.metrics.staleness.error.rate,
    );
    metrics.insert(
        "current_memory_contracts.injection.audit_injected".to_string(),
        current_memory_contracts
            .metrics
            .injection
            .audit_injected
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.injection.audit_dropped".to_string(),
        current_memory_contracts
            .metrics
            .injection
            .audit_dropped
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.injection.audit_abstained".to_string(),
        current_memory_contracts
            .metrics
            .injection
            .audit_abstained
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.injection.output_gate_recorded".to_string(),
        current_memory_contracts
            .metrics
            .injection
            .output_gate_recorded
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.usage.citation_event_matched".to_string(),
        current_memory_contracts
            .metrics
            .usage
            .citation_event_matched
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.usage.citation_event_no_citation".to_string(),
        current_memory_contracts
            .metrics
            .usage
            .citation_event_no_citation
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.usage.usage_event_linked_to_injection_item".to_string(),
        current_memory_contracts
            .metrics
            .usage
            .usage_event_linked_to_injection_item
            .rate,
    );
    metrics.insert(
        "current_memory_contracts.all_checks".to_string(),
        bool_metric(current_memory_contracts.metrics.all_checks_passed),
    );
    metrics.insert(
        "injection.expected_memory_recall".to_string(),
        injection.metrics.expected_memory_recall.rate,
    );
    metrics.insert(
        "injection.forbidden_memory_exclusion".to_string(),
        injection.metrics.forbidden_memory_exclusion.rate,
    );
    metrics.insert(
        "injection.abstention_false_positive_bound".to_string(),
        injection.metrics.abstention_false_positive_bound.rate,
    );
    metrics.insert(
        "injection.user_prompt_submit_memory_recall".to_string(),
        injection.metrics.user_prompt_submit_memory_recall.rate,
    );
    metrics.insert(
        "injection.user_prompt_submit_abstention_false_positive_bound".to_string(),
        injection
            .metrics
            .user_prompt_submit_abstention_false_positive_bound
            .rate,
    );
    metrics.insert(
        "injection.block_churn_unchanged".to_string(),
        injection.metrics.block_churn_unchanged.rate,
    );
    metrics.insert(
        "injection.block_churn_one_added_prefix_preserved".to_string(),
        injection
            .metrics
            .block_churn_one_added_prefix_preserved
            .rate,
    );
    metrics.insert(
        "injection.all_checks".to_string(),
        bool_metric(injection.metrics.all_checks_passed),
    );
    metrics.insert(
        "extraction.observation_precision".to_string(),
        extraction.metrics.observation_precision.rate,
    );
    metrics.insert(
        "extraction.observation_recall".to_string(),
        extraction.metrics.observation_recall.rate,
    );
    metrics.insert(
        "extraction.candidate_precision".to_string(),
        extraction.metrics.candidate_precision.rate,
    );
    metrics.insert(
        "extraction.candidate_recall".to_string(),
        extraction.metrics.candidate_recall.rate,
    );
    metrics.insert(
        "extraction.forbidden_observation_exclusion".to_string(),
        extraction.metrics.forbidden_observation_exclusion.rate,
    );
    metrics.insert(
        "extraction.forbidden_candidate_exclusion".to_string(),
        extraction.metrics.forbidden_candidate_exclusion.rate,
    );
    metrics.insert(
        "extraction.over_save_quality".to_string(),
        1.0 - extraction.metrics.over_save_penalty,
    );
    metrics.insert(
        "extraction.all_checks".to_string(),
        bool_metric(extraction.metrics.all_checks_passed),
    );
    metrics
}

fn insert_capacity_metrics(
    metrics: &mut BTreeMap<String, f64>,
    capacity: &crate::eval::capacity::CapacityEvalReport,
) {
    metrics.insert(
        "capacity.degradation.fused.recall_at_k_loss".to_string(),
        capacity.degradation.fused_recall_at_k_loss,
    );
    metrics.insert(
        "capacity.degradation.fused.ndcg_at_10_loss".to_string(),
        capacity.degradation.fused_ndcg_at_10_loss,
    );
    metrics.insert(
        "capacity.degradation.fused.evidence_recall_at_k_loss".to_string(),
        capacity.degradation.fused_evidence_recall_at_k_loss,
    );
    for (channel, degradation) in &capacity.degradation.channels {
        let prefix = format!("capacity.degradation.channel.{channel}");
        metrics.insert(
            format!("{prefix}.recall_at_k_loss"),
            degradation.recall_at_k_loss,
        );
        metrics.insert(
            format!("{prefix}.ndcg_at_10_loss"),
            degradation.ndcg_at_10_loss,
        );
        metrics.insert(
            format!("{prefix}.evidence_recall_at_k_loss"),
            degradation.evidence_recall_at_k_loss,
        );
    }
}

fn insert_golden_metrics(
    metrics: &mut BTreeMap<String, f64>,
    prefix: &str,
    values: &crate::eval::golden::MetricAverages,
) {
    metrics.insert(format!("{prefix}.hit_at_k"), values.hit_at_k);
    metrics.insert(format!("{prefix}.mrr_at_10"), values.mrr_at_10);
    metrics.insert(format!("{prefix}.precision_at_k"), values.precision_at_k);
    metrics.insert(format!("{prefix}.recall_at_k"), values.recall_at_k);
    metrics.insert(format!("{prefix}.ndcg_at_10"), values.ndcg_at_10);
    metrics.insert(
        format!("{prefix}.evidence_recall_at_k"),
        values.evidence_recall_at_k,
    );
}

fn bool_metric(value: bool) -> f64 {
    if value {
        1.0
    } else {
        0.0
    }
}

pub(crate) fn compare_metrics(
    baseline: &EvalGateBaseline,
    thresholds: &EvalGateThresholds,
    current: &BTreeMap<String, f64>,
) -> (Vec<EvalGateDelta>, Vec<String>) {
    let keys = baseline
        .metrics
        .keys()
        .chain(current.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut deltas = Vec::new();
    let mut failures = Vec::new();
    for key in keys {
        let threshold = thresholds.metrics.get(&key);
        let max_drop = threshold
            .map(|threshold| threshold.max_drop)
            .unwrap_or(thresholds.default_max_drop);
        let max_increase = threshold.and_then(|threshold| threshold.max_increase);
        match (baseline.metrics.get(&key), current.get(&key)) {
            (Some(expected), Some(actual)) => {
                let delta = actual - expected;
                let status = if let Some(max_increase) = max_increase {
                    if *actual > *expected + max_increase + f64::EPSILON {
                        failures.push(format!(
                            "{key} increased: baseline={expected:.4} current={actual:.4} max_increase={max_increase:.4}"
                        ));
                        EvalGateStatus::Fail
                    } else {
                        EvalGateStatus::Pass
                    }
                } else if actual + max_drop + f64::EPSILON < *expected {
                    failures.push(format!(
                        "{key} regressed: baseline={expected:.4} current={actual:.4} max_drop={max_drop:.4}"
                    ));
                    EvalGateStatus::Fail
                } else {
                    EvalGateStatus::Pass
                };
                let min_value = threshold.and_then(|threshold| threshold.min_value);
                let status = if let Some(min_value) = min_value {
                    if *actual <= min_value {
                        failures.push(format!(
                            "{key} below strict minimum: current={actual:.4} min_value={min_value:.4}"
                        ));
                        EvalGateStatus::Fail
                    } else {
                        status
                    }
                } else {
                    status
                };
                deltas.push(EvalGateDelta {
                    metric: key,
                    baseline: *expected,
                    current: *actual,
                    delta,
                    max_drop,
                    status,
                });
            }
            (Some(expected), None) => {
                failures.push(format!("{key} missing from current eval metrics"));
                deltas.push(EvalGateDelta {
                    metric: key,
                    baseline: *expected,
                    current: 0.0,
                    delta: -*expected,
                    max_drop,
                    status: EvalGateStatus::MissingCurrent,
                });
            }
            (None, Some(actual)) => {
                failures.push(format!("{key} missing from committed eval gate baseline"));
                deltas.push(EvalGateDelta {
                    metric: key,
                    baseline: 0.0,
                    current: *actual,
                    delta: *actual,
                    max_drop,
                    status: EvalGateStatus::MissingBaseline,
                });
            }
            (None, None) => {}
        }
    }
    (deltas, failures)
}

impl Display for EvalGateReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== remem eval-gates ===")?;
        writeln!(
            f,
            "baseline={} thresholds={} metrics={} passed={}",
            self.baseline_version,
            self.thresholds_version,
            self.summary.metrics_checked,
            self.summary.passed
        )?;
        writeln!(f)?;
        writeln!(
            f,
            "{:<58} {:>9} {:>9} {:>9} {:>9} status",
            "metric", "baseline", "current", "delta", "max_drop"
        )?;
        for delta in &self.deltas {
            writeln!(
                f,
                "{:<58} {:>9.4} {:>9.4} {:>9.4} {:>9.4} {}",
                delta.metric,
                delta.baseline,
                delta.current,
                delta.delta,
                delta.max_drop,
                delta.status.label()
            )?;
        }
        if !self.failures.is_empty() {
            writeln!(f)?;
            writeln!(f, "Failures:")?;
            for failure in &self.failures {
                writeln!(f, "- {failure}")?;
            }
        }
        Ok(())
    }
}

impl EvalGateStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::MissingCurrent => "MISSING_CURRENT",
            Self::MissingBaseline => "MISSING_BASELINE",
        }
    }
}

#[cfg(test)]
mod tests;
