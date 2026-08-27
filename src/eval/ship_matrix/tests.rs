use super::authority::{
    security_report_path_matches, supporting_report_binds_implementation,
    verify_executed_suite_identity, verify_security_platforms, verify_security_task_set,
    ClaimAuthority, SecurityAuthority,
};
use super::rows::{coding_gate, component_gate};
use super::scorecard::{build_scorecard, ratio_field, task_completion_counts, unavailable_field};
use super::*;
use crate::eval::gates::{EvalGateDelta, EvalGateStatus};
use sha2::Digest;

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
fn unreadable_security_evidence_is_incomplete_and_blocks_required_row() {
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
            claim_contract_path: temp.join("claim-contract.json"),
            input_artifact_sha256: BTreeMap::new(),
        },
    );
    let security = evidence
        .ship_matrix
        .gates
        .iter()
        .find(|gate| gate.id == "production_security_e2e")
        .unwrap();
    assert_eq!(security.status, ShipGateStatus::Incomplete);
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
        None,
    );
    assert_eq!(field.measurement_state, MeasurementState::Unavailable);
    assert_eq!(field.numerator.value, None);
    assert_eq!(field.denominator.value, None);
    assert!(!field.values.contains_key("rate"));
}

#[test]
fn partial_latency_evidence_is_fully_unavailable() {
    let public = PublicEvidence {
        report: None,
        report_error: None,
        security: Some(serde_json::json!({
            "aggregate_metrics": {
                "performance": {
                    "remem_default": {
                        "end_to_end_latency_p50_ms": 12.0,
                        "tasks": 20
                    }
                }
            }
        })),
        security_error: None,
        security_authority: SecurityAuthority {
            passed: true,
            benchmark_commit: None,
            diagnostics: Vec::new(),
        },
        claim_authority: ClaimAuthority {
            coding_passed: false,
            level3_passed: false,
            diagnostics: Vec::new(),
        },
    };
    let scorecard = build_scorecard(&public, Path::new(DEFAULT_SECURITY_REPORT));
    let latency = scorecard
        .fields
        .iter()
        .find(|field| field.id == "foreground_latency_p50_p95")
        .unwrap();
    assert_eq!(latency.measurement_state, MeasurementState::Unavailable);
    assert_eq!(latency.denominator.value, None);
    assert!(latency.values.is_empty());
}

#[test]
fn security_task_set_requires_every_suite_task() {
    let temp = unique_temp_dir("security-task-set");
    let suite_dir = temp.join("memory/suites/adversarial-policy");
    std::fs::create_dir_all(&suite_dir).unwrap();
    std::fs::write(
        suite_dir.join("suite.json"),
        br#"{"version":"v2","fixture_revision":"adversarial-policy-v2","tasks":[{"id":"one"},{"id":"two"}]}"#,
    )
    .unwrap();
    let options = ShipMatrixOptions {
        public_root: temp.clone(),
        ..Default::default()
    };
    let mut diagnostics = Vec::new();
    verify_security_task_set(
        &options,
        &std::collections::BTreeSet::from(["one".to_string()]),
        &mut diagnostics,
    );
    assert!(diagnostics
        .iter()
        .any(|item| item.contains("missing=[two]")));
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
fn executed_suite_identity_rejects_old_runs_after_same_id_runtime_edit() {
    let original = br#"{"tasks":[{"id":"same","prompt":"old","expected":"allow"}]}"#;
    let mutated = br#"{"tasks":[{"id":"same","prompt":"changed","expected":"deny"}]}"#;
    let original_identity = format!("sha256-raw-suite-v1:{:x}", sha2::Sha256::digest(original));
    let mut diagnostics = Vec::new();
    verify_executed_suite_identity(
        mutated,
        Some(&original_identity),
        &std::collections::BTreeSet::from([original_identity.clone()]),
        &mut diagnostics,
    );
    assert!(diagnostics
        .iter()
        .any(|item| item.contains("report suite content identity mismatch")));
    assert!(diagnostics
        .iter()
        .any(|item| item.contains("runs do not exactly bind")));
}

#[test]
fn selected_security_report_must_match_manifest_entry_path() {
    let temp = unique_temp_dir("security-report-binding");
    let reports = temp.join("memory/reports");
    std::fs::create_dir_all(&reports).unwrap();
    let selected = reports.join("selected.json");
    let other = reports.join("other.json");
    std::fs::write(&selected, b"{}").unwrap();
    std::fs::write(&other, b"{}").unwrap();
    let options = ShipMatrixOptions {
        public_root: temp.clone(),
        security_report_path: selected.clone(),
        ..Default::default()
    };
    let selected = selected.canonicalize().unwrap();
    assert!(security_report_path_matches(
        &options,
        "memory/reports/selected.json",
        &selected
    ));
    assert!(!security_report_path_matches(
        &options,
        "memory/reports/other.json",
        &selected
    ));
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
fn security_platform_must_exactly_match_current_os_and_arch() {
    let macos = std::collections::BTreeSet::from([("macos".to_string(), "aarch64".to_string())]);
    let mut diagnostics = Vec::new();
    verify_security_platforms(&macos, "linux", "x86_64", &mut diagnostics);
    assert!(diagnostics
        .iter()
        .any(|item| item.contains("does not exactly cover")));

    diagnostics.clear();
    verify_security_platforms(&macos, "macos", "aarch64", &mut diagnostics);
    assert!(diagnostics.is_empty());
}

#[test]
fn default_security_report_is_platform_specific() {
    assert_eq!(
        security_report_for_platform("macos", "aarch64"),
        PathBuf::from(DEFAULT_SECURITY_REPORT)
    );
    assert_eq!(
        security_report_for_platform("linux", "x86_64"),
        PathBuf::from(LINUX_X86_64_SECURITY_REPORT)
    );
    assert_eq!(
        security_report_for_platform("windows", "x86_64"),
        PathBuf::from("eval/public/memory/reports/adversarial-policy-v2-windows-x86_64.json")
    );
}

#[test]
fn legacy_gate_failure_blocks_merge_and_release_readiness() {
    let gates = vec![
        test_gate("merge", ShipGateStatus::Pass, vec!["merge"], true),
        test_gate("release", ShipGateStatus::Pass, vec!["release"], true),
    ];
    let legacy_gates_passed = false;
    let implementation_identified = true;
    let source_clean = true;
    assert!(!merge_is_ready(&gates, legacy_gates_passed));
    assert!(!release_is_ready(
        &gates,
        legacy_gates_passed,
        implementation_identified,
        source_clean
    ));
}

#[test]
fn supporting_claim_report_must_bind_current_sha() {
    let current = "0123456789abcdef0123456789abcdef01234567";
    let stale = serde_json::to_vec(&serde_json::json!({
        "reproducibility": {
            "remem_commits": ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]
        }
    }))
    .unwrap();
    let mut diagnostics = Vec::new();
    assert!(!supporting_report_binds_implementation(
        &stale,
        "claim",
        Some(current),
        Some(&"a".repeat(64)),
        &mut diagnostics,
    ));
    assert!(diagnostics
        .iter()
        .any(|item| item.contains("production source is not equivalent")));
}

#[test]
fn supporting_claim_report_accepts_source_equivalent_producing_revision() {
    let current = "0123456789abcdef0123456789abcdef01234567";
    let producing = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let tree = "b".repeat(64);
    let report = serde_json::to_vec(&serde_json::json!({
        "reproducibility": {
            "remem_commits": [producing],
            "production_input_tree_sha256": tree,
        }
    }))
    .unwrap();
    let mut diagnostics = Vec::new();
    assert!(supporting_report_binds_implementation(
        &report,
        "claim",
        Some(current),
        Some(&"b".repeat(64)),
        &mut diagnostics,
    ));
    assert!(diagnostics.is_empty());
}

#[test]
fn serialized_gate_always_carries_exclusions() {
    let value =
        serde_json::to_value(test_gate("gate", ShipGateStatus::Pass, vec!["merge"], true)).unwrap();
    assert_eq!(value["exclusions"], serde_json::json!([]));
}

#[test]
fn missing_capacity_evidence_is_incomplete_and_blocks_command() {
    let temp = unique_temp_dir("capacity-incomplete");
    std::fs::create_dir_all(&temp).unwrap();
    let artifact = temp.join("artifact.json");
    std::fs::write(&artifact, b"{}").unwrap();
    let evidence = build_ship_evidence(
        &[],
        false,
        true,
        ShipMatrixOptions {
            baseline_path: artifact.to_string_lossy().to_string(),
            thresholds_path: artifact.to_string_lossy().to_string(),
            golden_dataset_path: artifact.to_string_lossy().to_string(),
            public_root: temp.join("public"),
            security_report_path: temp.join("security.json"),
            cross_host_charter_path: temp.join("charter.json"),
            claim_registry_path: temp.join("claims.json"),
            claim_contract_path: temp.join("claim-contract.json"),
            input_artifact_sha256: BTreeMap::new(),
        },
    );
    let capacity = evidence
        .ship_matrix
        .gates
        .iter()
        .find(|gate| gate.id == "capacity")
        .unwrap();
    assert_eq!(capacity.status, ShipGateStatus::Incomplete);
    assert!(!evidence.ship_matrix.summary.command_passed);
    std::fs::remove_dir_all(temp).unwrap();
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
    let scorecard = build_scorecard(&public, Path::new(DEFAULT_SECURITY_REPORT));
    let security = scorecard
        .fields
        .iter()
        .find(|field| field.id == "poison_policy_leak_rate")
        .unwrap();
    assert_eq!(security.measurement_state, MeasurementState::Unavailable);
    assert_eq!(security.denominator.value, None);
}

#[test]
fn synthetic_latency_is_not_reported_as_measured() {
    let public = PublicEvidence {
        report: None,
        report_error: None,
        security: Some(serde_json::json!({
            "aggregate_metrics": {
                "performance": {
                    "remem_default": {
                        "end_to_end_latency_p50_ms": 10.0,
                        "end_to_end_latency_p95_ms": 20.0,
                        "tasks": 20
                    }
                }
            }
        })),
        security_error: None,
        security_authority: SecurityAuthority {
            passed: true,
            benchmark_commit: None,
            diagnostics: Vec::new(),
        },
        claim_authority: ClaimAuthority {
            coding_passed: false,
            level3_passed: false,
            diagnostics: Vec::new(),
        },
    };
    let selected = Path::new(LINUX_X86_64_SECURITY_REPORT);
    let scorecard = build_scorecard(&public, selected);
    let latency = scorecard
        .fields
        .iter()
        .find(|field| field.id == "foreground_latency_p50_p95")
        .unwrap();
    assert_eq!(latency.measurement_state, MeasurementState::Unavailable);
    assert_eq!(latency.source, None);
    assert!(latency.note.contains("synthetic"));
}

#[test]
fn task_completion_excludes_runs_that_never_started_the_target() {
    let (resolved, started) = task_completion_counts(
        [
            (Some(true), true),
            (Some(true), false),
            (Some(false), true),
            (None, true),
        ]
        .into_iter(),
    );

    assert_eq!(resolved, 1.0);
    assert_eq!(started, 2.0);
}

#[test]
fn measured_security_ratio_binds_the_exact_selected_artifact() {
    let temp = unique_temp_dir("scorecard-source");
    std::fs::create_dir_all(&temp).unwrap();
    let selected = temp.join("security.json");
    let selected_bytes = br#"{"aggregate_metrics":{"policy":{"non_retention_cases":4,"non_retention_leak_rate":0.0}}}"#;
    std::fs::write(&selected, selected_bytes).unwrap();
    let public = PublicEvidence {
        report: None,
        report_error: None,
        security: Some(serde_json::from_slice(selected_bytes).unwrap()),
        security_error: None,
        security_authority: SecurityAuthority {
            passed: true,
            benchmark_commit: None,
            diagnostics: Vec::new(),
        },
        claim_authority: ClaimAuthority {
            coding_passed: false,
            level3_passed: false,
            diagnostics: Vec::new(),
        },
    };
    let scorecard = build_scorecard(&public, &selected);
    let security = scorecard
        .fields
        .iter()
        .find(|field| field.id == "poison_policy_leak_rate")
        .unwrap();
    let expected = format!(
        "{}#sha256={:x}",
        selected.to_string_lossy(),
        Sha256::digest(selected_bytes)
    );
    assert_eq!(security.measurement_state, MeasurementState::Measured);
    assert_eq!(security.source.as_deref(), Some(expected.as_str()));
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
fn measured_memory_help_ratio_binds_its_eligible_artifacts() {
    let report = crate::eval::bench_artifact::generate_public_baseline_report(Path::new(
        DEFAULT_PUBLIC_ROOT,
    ))
    .unwrap();
    let public = PublicEvidence {
        report: Some(report),
        report_error: None,
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

    let scorecard = build_scorecard(&public, Path::new(DEFAULT_SECURITY_REPORT));
    let memory_help = scorecard
        .fields
        .iter()
        .find(|field| field.id == "correct_memory_help_rate")
        .unwrap();

    assert_eq!(memory_help.measurement_state, MeasurementState::Measured);
    assert!(memory_help
        .source
        .as_deref()
        .is_some_and(|source| source.contains("coding/reports/coding-report-v1.json#sha256=")));
}

#[test]
fn evaluated_evidence_rejects_a_file_changed_after_load() {
    let temp = unique_temp_dir("evaluated-bytes");
    std::fs::create_dir_all(&temp).unwrap();
    let artifact = temp.join("baseline.json");
    let consumed = br#"{"metric":1}"#;
    std::fs::write(&artifact, consumed).unwrap();
    let consumed_sha256 = format!("{:x}", Sha256::digest(consumed));
    std::fs::write(&artifact, br#"{"metric":2}"#).unwrap();

    let evidence = evidence_for_evaluated_path(&artifact, Some(&consumed_sha256));

    assert_eq!(evidence.state, ArtifactState::Invalid);
    assert_eq!(evidence.sha256.as_deref(), Some(consumed_sha256.as_str()));
    assert!(evidence.detail.contains("changed after eval-gates loaded"));
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
fn coding_baseline_evidence_is_unverified_when_artifact_verifier_fails() {
    let mut report = crate::eval::bench_artifact::generate_public_baseline_report(Path::new(
        DEFAULT_PUBLIC_ROOT,
    ))
    .unwrap();
    report.artifact_verifier.passed = false;
    let public = PublicEvidence {
        report: Some(report),
        report_error: None,
        security: None,
        security_error: None,
        security_authority: SecurityAuthority {
            passed: false,
            benchmark_commit: None,
            diagnostics: Vec::new(),
        },
        claim_authority: ClaimAuthority {
            coding_passed: true,
            level3_passed: false,
            diagnostics: Vec::new(),
        },
    };
    let implementation = ImplementationIdentity {
        git_sha: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        checkout_git_sha: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        source_dirty: Some(false),
        production_input_tree_sha256: Some("a".repeat(64)),
        checkout_production_input_tree_sha256: Some("a".repeat(64)),
        executable_source_equivalent: true,
        package_version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    };
    let gate = coding_gate(&ShipMatrixOptions::default(), &public, &implementation);
    assert_eq!(gate.status, ShipGateStatus::Unavailable);
    assert_eq!(gate.evidence[0].state, ArtifactState::Invalid);
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

fn unique_temp_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "remem-ship-matrix-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
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
        exclusions: Vec::new(),
        evidence: Vec::new(),
        diagnostics: Vec::new(),
    }
}
