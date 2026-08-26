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
                min_value: None,
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
                min_value: None,
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
                    min_value: None,
                },
            ),
            (
                "golden.slice.temporal.hit_at_k".to_string(),
                EvalGateThreshold {
                    max_drop: 0.05,
                    max_increase: None,
                    min_value: None,
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

#[test]
fn text_ship_summary_prints_the_combined_command_verdict() {
    let summary = crate::eval::ship_matrix::ShipMatrixSummary {
        command_passed: false,
        merge_ready: false,
        release_ready: false,
        implementation_identified: true,
        source_clean: true,
        default_on_ready: false,
        cross_host_claim_ready: false,
        coding_outcome_claim_ready: false,
        public_claim_ready: false,
    };
    let rendered = format_ship_summary(&summary);
    assert!(rendered.contains("command_passed=false"));
}

#[test]
fn gate_blocks_zero_metric_when_min_value_requires_strictly_positive() {
    let key = "golden.slice.paraphrase.hit_at_k".to_string();
    let baseline = EvalGateBaseline {
        version: "test".to_string(),
        metrics: BTreeMap::from([(key.clone(), 0.0)]),
    };
    let thresholds = EvalGateThresholds {
        version: "test".to_string(),
        default_max_drop: 0.0,
        metrics: BTreeMap::from([(
            key.clone(),
            EvalGateThreshold {
                max_drop: 0.0,
                max_increase: None,
                min_value: Some(0.0),
            },
        )]),
    };

    // A zero current value passes the no-drop check but must fail min_value.
    let current = BTreeMap::from([(key.clone(), 0.0)]);
    let (deltas, failures) = compare_metrics(&baseline, &thresholds, &current);
    assert_eq!(deltas[0].status, EvalGateStatus::Fail);
    assert!(failures[0].contains("below strict minimum"));

    // A strictly positive value passes.
    let current = BTreeMap::from([(key, 0.25)]);
    let (deltas, failures) = compare_metrics(&baseline, &thresholds, &current);
    assert_eq!(deltas[0].status, EvalGateStatus::Pass);
    assert!(failures.is_empty());
}
