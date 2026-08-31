use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use super::super::rows::coding_gate;
use super::super::scorecard::build_scorecard;
use super::super::*;
use super::test_gate;
use crate::eval::bench_artifact::{AuthorityStatus, PublicBaselineReport};

fn baseline_fixture() -> PublicBaselineReport {
    static BASELINE: OnceLock<PublicBaselineReport> = OnceLock::new();
    BASELINE
        .get_or_init(|| {
            crate::eval::bench_artifact::generate_public_baseline_report(
                Path::new(DEFAULT_PUBLIC_ROOT),
                Path::new(DEFAULT_CLAIM_REGISTRY),
            )
            .expect("generate verified public baseline")
        })
        .clone()
}

fn public_fixture(mut report: PublicBaselineReport) -> PublicEvidence {
    let selected_relative = Path::new(DEFAULT_SECURITY_REPORT)
        .strip_prefix(DEFAULT_PUBLIC_ROOT)
        .expect("default security report is below public root")
        .to_string_lossy()
        .to_string();
    let selected = report
        .artifact_verifier
        .verified_artifacts
        .reports
        .iter()
        .find(|artifact| artifact.path == selected_relative)
        .expect("selected security report is verified")
        .clone();
    let source = format!("{DEFAULT_SECURITY_REPORT}#sha256={}", selected.sha256);
    let security_authority = report
        .artifact_verifier
        .authority_verdict
        .security
        .reports
        .iter()
        .find(|authority| authority.report_path == selected_relative)
        .expect("selected security authority")
        .clone();
    let gh931 = &mut report.artifact_verifier.authority_verdict.gh931;
    gh931.status = AuthorityStatus::Insufficient;
    gh931.registry.declared_statuses = vec![AuthorityStatus::Pass; 3];
    for claim in &mut gh931.claims {
        claim.declared_registry_status = AuthorityStatus::Pass;
    }
    if let Some(registry) = &mut report.artifact_verifier.verified_artifacts.claim_registry {
        for claim in &mut registry.value.claims {
            claim.status = AuthorityStatus::Pass;
            claim.supporting_report = serde_json::json!({
                "path": "eval/public/reports/tampered.json",
                "sha256": "f".repeat(64)
            });
        }
    }
    PublicEvidence {
        report: Some(report),
        report_error: None,
        security_claim_level: Some(selected.value.claim_level),
        security_source: Some(source),
        security_error: None,
        security_authority: Some(security_authority),
    }
}

#[test]
fn tampered_registry_pass_and_supporting_report_cannot_authorize_coding_row() {
    let public = public_fixture(baseline_fixture());

    let row = coding_gate(&ShipMatrixOptions::default(), &public);

    assert_eq!(row.status, ShipGateStatus::Unavailable);
    assert_eq!(row.claim_level, "unavailable_no_authorized_coding_claim");
}

#[test]
fn tampered_security_aggregate_cannot_make_scorecard_measured() {
    let mut report = baseline_fixture();
    let selected_relative = Path::new(DEFAULT_SECURITY_REPORT)
        .strip_prefix(DEFAULT_PUBLIC_ROOT)
        .expect("default security report is below public root")
        .to_string_lossy();
    let selected_authority = report
        .artifact_verifier
        .authority_verdict
        .security
        .reports
        .iter_mut()
        .find(|authority| authority.report_path == selected_relative)
        .expect("selected security authority exists");
    selected_authority.status = AuthorityStatus::Fail;
    selected_authority
        .diagnostics
        .push("recomputed aggregate mismatch".to_string());
    let public = public_fixture(report);

    let scorecard = build_scorecard(&public, Path::new(DEFAULT_SECURITY_REPORT));
    let leak_rate = scorecard
        .fields
        .iter()
        .find(|field| field.id == "poison_policy_leak_rate")
        .expect("security scorecard field");

    assert_eq!(leak_rate.measurement_state, MeasurementState::Unavailable);
    assert_eq!(leak_rate.numerator.value, None);
    assert_eq!(leak_rate.denominator.value, None);
}

#[test]
fn selected_security_row_uses_report_local_model_and_platform() {
    let mut report = baseline_fixture();
    report.reproducibility.models =
        vec!["global/union-a".to_string(), "global/union-b".to_string()];
    let public = public_fixture(report);
    let selected = public
        .report
        .as_ref()
        .expect("baseline")
        .artifact_verifier
        .authority_verdict
        .security
        .reports
        .iter()
        .find(|authority| authority.report_path == "memory/reports/adversarial-policy-v2.json")
        .expect("selected security authority");

    let row =
        super::super::rows::build_gate_rows(&[], false, &ShipMatrixOptions::default(), &public)
            .into_iter()
            .find(|row| row.id == "production_security_e2e")
            .expect("security row");

    let expected_models = selected
        .models
        .iter()
        .map(|model| serde_json::to_string(model).expect("serialize model identity"))
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(row.model_identity, expected_models);
    assert_eq!(row.platform_identity, selected.target.clone().unwrap());
    assert_ne!(row.model_identity, "global/union-a,global/union-b");
}

#[test]
fn stale_security_report_tree_cannot_pass_current_implementation_gate() {
    let mut public = public_fixture(baseline_fixture());
    public
        .security_authority
        .as_mut()
        .expect("selected security authority")
        .production_input_trees = vec!["f".repeat(64)];

    let row =
        super::super::rows::build_gate_rows(&[], false, &ShipMatrixOptions::default(), &public)
            .into_iter()
            .find(|row| row.id == "production_security_e2e")
            .expect("security row");

    assert_eq!(row.status, ShipGateStatus::Incomplete);
    assert!(row
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("current verifier implementation tree")));
}

#[test]
fn release_readiness_requires_verifier_release_and_legacy_gates() {
    let gates = vec![test_gate(
        "release",
        ShipGateStatus::Pass,
        vec!["release"],
        true,
    )];

    assert!(!release_is_ready(&gates, true, false));
    assert!(!release_is_ready(&gates, false, true));
    assert!(release_is_ready(&gates, true, true));
}

#[test]
fn scorecard_projects_only_remem_e2e_completion_from_verdict() {
    let mut report = baseline_fixture();
    let gh931 = &mut report.artifact_verifier.authority_verdict.gh931;
    gh931.completeness.complete = true;
    gh931.completeness.attempts_ready = true;
    gh931.completeness.machine_outcomes_ready = true;
    gh931.report = Some(crate::eval::bench_artifact::Gh931ReportBinding {
        path: "coding/reports/official.json".to_string(),
        sha256: "d".repeat(64),
        conditions: vec![
            "no_memory".to_string(),
            "remem_e2e".to_string(),
            "curated_file_budgeted".to_string(),
        ],
        models_by_condition: BTreeMap::new(),
        platforms: vec!["linux/x86_64".to_string()],
        producing_shas: vec!["a".repeat(40)],
        production_input_trees: vec!["b".repeat(64)],
        source_dirty_attestations: vec![Some(false)],
    });
    for completion in &mut gh931.condition_completion {
        match completion.condition.as_str() {
            "remem_e2e" => {
                completion.eligible_started = 2;
                completion.resolved = 1;
            }
            _ => {
                completion.eligible_started = 48;
                completion.resolved = 48;
            }
        }
    }
    for outcome in &mut report.coding_task_outcomes {
        outcome.condition = "no_memory".to_string();
        outcome.target_started = Some(true);
        outcome.resolved = true;
    }
    let public = public_fixture(report);

    let scorecard = build_scorecard(&public, Path::new(DEFAULT_SECURITY_REPORT));
    let completion = scorecard
        .fields
        .iter()
        .find(|field| field.id == "task_completion_rate")
        .expect("task completion field");

    assert_eq!(completion.measurement_state, MeasurementState::Measured);
    assert_eq!(completion.numerator.value, Some(1.0));
    assert_eq!(completion.denominator.value, Some(2.0));
    assert!(completion.eligible_population.contains("remem_e2e"));
    assert_eq!(
        completion.source.as_deref(),
        Some("coding/reports/official.json#sha256=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd")
    );
}

#[test]
fn scorecard_keeps_completion_unavailable_without_complete_machine_outcomes() {
    let mut report = baseline_fixture();
    let gh931 = &mut report.artifact_verifier.authority_verdict.gh931;
    gh931.completeness.complete = true;
    gh931.completeness.attempts_ready = true;
    gh931.completeness.machine_outcomes_ready = false;
    let completion = gh931
        .condition_completion
        .iter_mut()
        .find(|completion| completion.condition == "remem_e2e")
        .expect("remem_e2e completion");
    completion.eligible_started = 48;
    completion.resolved = 47;

    let public = public_fixture(report);
    let scorecard = build_scorecard(&public, Path::new(DEFAULT_SECURITY_REPORT));
    let completion = scorecard
        .fields
        .iter()
        .find(|field| field.id == "task_completion_rate")
        .expect("task completion field");

    assert_eq!(completion.measurement_state, MeasurementState::Unavailable);
    assert_eq!(completion.numerator.value, None);
    assert_eq!(completion.denominator.value, None);
}
