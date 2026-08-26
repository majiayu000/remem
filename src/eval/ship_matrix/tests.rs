use super::authority::{ClaimAuthority, SecurityAuthority};
use super::rows::{claim_artifact_matches_implementation, component_gate};
use super::scorecard::{build_scorecard, ratio_field, unavailable_field};
use super::*;
use crate::eval::gates::{EvalGateDelta, EvalGateStatus};

fn delta(metric: &str, status: EvalGateStatus) -> EvalGateDelta {
    EvalGateDelta {
        metric: metric.to_string(),
        baseline: 1.0,
        current: if status == EvalGateStatus::Pass {
            1.0
        } else {
            0.0
        },
        delta: if status == EvalGateStatus::Pass {
            0.0
        } else {
            -1.0
        },
        max_drop: 0.0,
        status,
    }
}

#[test]
fn component_gate_never_turns_missing_metrics_into_pass() {
    let gate = component_gate(
        "deterministic_retrieval",
        "test",
        &["golden."],
        &metric_set_hash(&[]),
        &[],
        Vec::new(),
        vec!["merge"],
    );
    assert_eq!(gate.status, ShipGateStatus::Incomplete);
    assert!(!pass_or_not_applicable(&gate));
}

#[test]
fn component_gate_propagates_failures() {
    let deltas = [
        delta("golden.overall.hit_at_k", EvalGateStatus::Pass),
        delta("golden.slice.temporal.hit_at_k", EvalGateStatus::Fail),
    ];
    let gate = component_gate(
        "deterministic_retrieval",
        "test",
        &["golden."],
        &metric_set_hash(&["golden.overall.hit_at_k", "golden.slice.temporal.hit_at_k"]),
        &deltas,
        Vec::new(),
        vec!["merge"],
    );
    assert_eq!(gate.status, ShipGateStatus::Fail);
    assert_eq!(gate.metric_deltas.len(), 2);
}

#[test]
fn missing_component_artifact_cannot_pass() {
    let gate = component_gate(
        "deterministic_retrieval",
        "test",
        &["golden."],
        &metric_set_hash(&["golden.overall.hit_at_k"]),
        &[delta("golden.overall.hit_at_k", EvalGateStatus::Pass)],
        vec![ArtifactEvidence {
            path: "missing.json".to_string(),
            state: ArtifactState::Missing,
            sha256: None,
            detail: "missing".to_string(),
        }],
        vec!["merge"],
    );
    assert_eq!(gate.status, ShipGateStatus::Fail);
    assert!(gate.diagnostics[0].contains("required evidence"));
}

#[test]
fn component_gate_rejects_a_shrunken_metric_set() {
    let gate = component_gate(
        "deterministic_retrieval",
        "test",
        &["golden."],
        &metric_set_hash(&["golden.overall.hit_at_k", "golden.overall.mrr_at_10"]),
        &[delta("golden.overall.hit_at_k", EvalGateStatus::Pass)],
        Vec::new(),
        vec!["merge"],
    );
    assert_eq!(gate.status, ShipGateStatus::Incomplete);
    assert!(gate
        .diagnostics
        .iter()
        .any(|item| item.contains("metric set identity mismatch")));
}

#[test]
fn invalid_public_evidence_blocks_required_security_row() {
    let temp = std::env::temp_dir().join(format!(
        "remem-ship-matrix-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let invalid_security = temp.join("security.json");
    std::fs::write(&invalid_security, b"{").unwrap();
    let charter = temp.join("charter.json");
    std::fs::write(&charter, br#"{"status":"PASS","executable_ready":true}"#).unwrap();
    let claims = temp.join("claims.json");
    std::fs::write(&claims, b"{}").unwrap();
    let artifact = temp.join("component.json");
    std::fs::write(&artifact, b"{}").unwrap();
    let deltas = vec![
        delta("golden.overall.hit_at_k", EvalGateStatus::Pass),
        delta("capacity.fused.recall", EvalGateStatus::Pass),
        delta("injection.all_checks", EvalGateStatus::Pass),
        delta("current_memory_contracts.all_checks", EvalGateStatus::Pass),
    ];
    let evidence = build_ship_evidence(
        &deltas,
        true,
        true,
        ShipMatrixOptions {
            baseline_path: artifact.to_string_lossy().to_string(),
            thresholds_path: artifact.to_string_lossy().to_string(),
            golden_dataset_path: artifact.to_string_lossy().to_string(),
            public_root: temp.join("empty-public-root"),
            security_report_path: invalid_security,
            cross_host_charter_path: charter,
            claim_registry_path: claims,
        },
    );
    let security = evidence
        .ship_matrix
        .gates
        .iter()
        .find(|gate| gate.id == "production_security_e2e")
        .unwrap();
    assert_eq!(security.status, ShipGateStatus::Fail);
    let cross_host = evidence
        .ship_matrix
        .gates
        .iter()
        .find(|gate| gate.id == "cross_host")
        .unwrap();
    assert_eq!(cross_host.status, ShipGateStatus::Unavailable);
    assert!(!evidence.ship_matrix.summary.command_passed);
    assert!(!evidence.ship_matrix.summary.default_on_ready);
    assert!(!evidence.ship_matrix.summary.public_claim_ready);
    std::fs::remove_dir_all(&temp).unwrap();
}

#[test]
fn unavailable_scorecard_fields_have_no_synthetic_zero() {
    let field = unavailable_field(
        "repeated_explanation_rate",
        "eligible population",
        "events",
        "opportunities",
        "not measured",
    );
    assert_eq!(field.measurement_state, MeasurementState::Unavailable);
    assert_eq!(field.numerator.value, None);
    assert_eq!(field.denominator.value, None);
    assert!(field.values.is_empty());
}

#[test]
fn ratio_requires_a_nonzero_denominator() {
    let field = ratio_field(
        "task_completion_rate",
        Some(0.0),
        Some(0.0),
        "population",
        "resolved",
        "runs",
        "threshold",
        "no_claim",
    );
    assert_eq!(field.measurement_state, MeasurementState::Unavailable);
    assert_eq!(field.numerator.value, None);
    assert_eq!(field.denominator.value, None);
    assert!(!field.values.contains_key("rate"));
}

#[test]
fn unavailable_claim_scope_does_not_fail_merge_scope() {
    let merge = test_gate("merge", ShipGateStatus::Pass, vec!["merge"], true);
    let claim = test_gate(
        "claim",
        ShipGateStatus::Unavailable,
        vec!["public_claim"],
        false,
    );
    let gates = vec![merge, claim];
    assert!(required_rows_pass(&gates, "merge"));
    assert!(!gate_passes(&gates, "public_claim"));
    assert!(gates
        .iter()
        .filter(|gate| gate.required_for_command_success)
        .all(pass_or_not_applicable));
}

#[test]
fn absent_claim_report_cannot_match_current_implementation() {
    let public = PublicEvidence {
        report: None,
        report_error: Some("missing".to_string()),
        security: None,
        security_error: None,
        security_authority: SecurityAuthority {
            passed: false,
            benchmark_commit: None,
            diagnostics: Vec::new(),
        },
        claim_authority: ClaimAuthority {
            coding_passed: false,
            level3_passed: false,
            diagnostics: Vec::new(),
        },
    };
    let implementation = ImplementationIdentity {
        git_sha: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        source_dirty: Some(false),
        package_version: "test",
        os: "test",
        arch: "test",
    };
    assert!(!claim_artifact_matches_implementation(
        &public,
        &implementation
    ));
}

#[test]
fn unverified_security_authority_hides_security_scorecard_numbers() {
    let public = PublicEvidence {
        report: None,
        report_error: Some("missing".to_string()),
        security: Some(serde_json::json!({
            "aggregate_metrics": {
                "policy": {
                    "non_retention_cases": 4,
                    "non_retention_leak_rate": 0.0
                }
            }
        })),
        security_error: None,
        security_authority: SecurityAuthority {
            passed: false,
            benchmark_commit: None,
            diagnostics: vec!["invalid".to_string()],
        },
        claim_authority: ClaimAuthority {
            coding_passed: false,
            level3_passed: false,
            diagnostics: Vec::new(),
        },
    };
    let scorecard = build_scorecard(&public);
    let security = scorecard
        .fields
        .iter()
        .find(|field| field.id == "poison_policy_leak_rate")
        .unwrap();
    assert_eq!(security.measurement_state, MeasurementState::Unavailable);
    assert_eq!(security.denominator.value, None);
}

fn metric_set_hash(metrics: &[&str]) -> String {
    use sha2::{Digest, Sha256};

    let mut metrics = metrics.to_vec();
    metrics.sort_unstable();
    let mut hasher = Sha256::new();
    for metric in metrics {
        hasher.update(metric.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

fn test_gate(
    id: &'static str,
    status: ShipGateStatus,
    blocks: Vec<&'static str>,
    required_for_command_success: bool,
) -> ShipGateRow {
    ShipGateRow {
        id,
        owner: "test",
        status,
        blocks,
        required_for_command_success,
        claim_level: "no_claim".to_string(),
        condition_completeness: "test".to_string(),
        config_identity: "test".to_string(),
        model_identity: "test".to_string(),
        metric_deltas: BTreeMap::new(),
        stop_loss_verdict: "test".to_string(),
        evidence: Vec::new(),
        diagnostics: Vec::new(),
    }
}
