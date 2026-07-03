pub mod associative;
pub mod bench_artifact;
pub mod capacity;
pub mod coding_bench;
pub mod current_memory_contracts;
pub mod e2e;
pub mod extraction;
pub mod memory_bench;
pub mod gates {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt::{self, Display};
    use std::fs;
    use std::path::Path;

    use anyhow::{Context, Result};
    use serde::{Deserialize, Serialize};

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

    pub fn run_eval_gates(options: EvalGateOptions) -> Result<EvalGateReport> {
        let mut baseline = load_baseline(&options.baseline_path)?;
        let mut thresholds = load_thresholds(&options.thresholds_path)?;
        let golden_dataset = crate::eval::golden::load_dataset(&options.golden_dataset_path)?;
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
        })
    }

    fn load_baseline(path: &str) -> Result<EvalGateBaseline> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("read eval gate baseline {}", Path::new(path).display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("parse eval gate baseline {}", Path::new(path).display()))
    }

    fn load_thresholds(path: &str) -> Result<EvalGateThresholds> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("read eval gate thresholds {}", Path::new(path).display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("parse eval gate thresholds {}", Path::new(path).display()))
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
    mod tests {
        use super::*;

        #[test]
        fn gate_blocks_constructed_retrieval_regression() {
            let baseline = EvalGateBaseline {
                version: "test".to_string(),
                metrics: BTreeMap::from([("golden.slice.temporal.hit_at_k".to_string(), 1.0)]),
            };
            let thresholds = EvalGateThresholds {
                version: "test".to_string(),
                default_max_drop: 0.05,
                metrics: BTreeMap::new(),
            };
            let current = BTreeMap::from([("golden.slice.temporal.hit_at_k".to_string(), 0.80)]);

            let (deltas, failures) = compare_metrics(&baseline, &thresholds, &current);

            assert_eq!(deltas[0].status, EvalGateStatus::Fail);
            assert_eq!(failures.len(), 1);
            assert!(failures[0].contains("golden.slice.temporal.hit_at_k regressed"));
        }

        #[test]
        fn gate_blocks_constructed_capacity_loss_increase() {
            let baseline = EvalGateBaseline {
                version: "test".to_string(),
                metrics: BTreeMap::from([(
                    "capacity.degradation.fused.recall_at_k_loss".to_string(),
                    0.0,
                )]),
            };
            let thresholds = EvalGateThresholds {
                version: "test".to_string(),
                default_max_drop: 0.0,
                metrics: BTreeMap::from([(
                    "capacity.degradation.fused.recall_at_k_loss".to_string(),
                    EvalGateThreshold {
                        max_drop: 0.0,
                        max_increase: Some(0.05),
                    },
                )]),
            };
            let current = BTreeMap::from([(
                "capacity.degradation.fused.recall_at_k_loss".to_string(),
                0.10,
            )]);

            let (deltas, failures) = compare_metrics(&baseline, &thresholds, &current);

            assert_eq!(deltas[0].status, EvalGateStatus::Fail);
            assert_eq!(failures.len(), 1);
            assert!(failures[0].contains("capacity.degradation.fused.recall_at_k_loss increased"));
        }

        #[test]
        fn gate_allows_constructed_capacity_loss_improvement() {
            let baseline = EvalGateBaseline {
                version: "test".to_string(),
                metrics: BTreeMap::from([(
                    "capacity.degradation.fused.recall_at_k_loss".to_string(),
                    0.10,
                )]),
            };
            let thresholds = EvalGateThresholds {
                version: "test".to_string(),
                default_max_drop: 0.0,
                metrics: BTreeMap::from([(
                    "capacity.degradation.fused.recall_at_k_loss".to_string(),
                    EvalGateThreshold {
                        max_drop: 0.0,
                        max_increase: Some(0.05),
                    },
                )]),
            };
            let current = BTreeMap::from([(
                "capacity.degradation.fused.recall_at_k_loss".to_string(),
                0.05,
            )]);

            let (deltas, failures) = compare_metrics(&baseline, &thresholds, &current);

            assert_eq!(deltas[0].status, EvalGateStatus::Pass);
            assert!(failures.is_empty());
        }

        #[test]
        fn skipped_capacity_gate_removes_capacity_metrics() {
            let mut baseline = EvalGateBaseline {
                version: "test".to_string(),
                metrics: BTreeMap::from([
                    (
                        "capacity.degradation.fused.recall_at_k_loss".to_string(),
                        0.0,
                    ),
                    ("golden.slice.temporal.hit_at_k".to_string(), 1.0),
                ]),
            };
            let mut thresholds = EvalGateThresholds {
                version: "test".to_string(),
                default_max_drop: 0.0,
                metrics: BTreeMap::from([
                    (
                        "capacity.degradation.fused.recall_at_k_loss".to_string(),
                        EvalGateThreshold {
                            max_drop: 0.0,
                            max_increase: Some(0.05),
                        },
                    ),
                    (
                        "golden.slice.temporal.hit_at_k".to_string(),
                        EvalGateThreshold {
                            max_drop: 0.05,
                            max_increase: None,
                        },
                    ),
                ]),
            };

            remove_capacity_gate_metrics(&mut baseline, &mut thresholds);

            assert!(!baseline
                .metrics
                .contains_key("capacity.degradation.fused.recall_at_k_loss"));
            assert!(!thresholds
                .metrics
                .contains_key("capacity.degradation.fused.recall_at_k_loss"));
            assert!(baseline
                .metrics
                .contains_key("golden.slice.temporal.hit_at_k"));
            assert!(thresholds
                .metrics
                .contains_key("golden.slice.temporal.hit_at_k"));
        }

        #[test]
        fn gate_report_table_status_labels_are_stable() {
            assert_eq!(EvalGateStatus::Pass.label(), "PASS");
            assert_eq!(EvalGateStatus::Fail.label(), "FAIL");
            assert_eq!(EvalGateStatus::MissingCurrent.label(), "MISSING_CURRENT");
            assert_eq!(EvalGateStatus::MissingBaseline.label(), "MISSING_BASELINE");
        }
    }
}
pub mod golden;
pub mod governance;
pub mod graph_decision;
pub mod injection;
pub mod local;
pub mod metrics;
pub mod weight_grid;
