use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{verify_benchmark_artifacts, BenchVerifyOptions};

mod schema;
mod security_closed_world;
mod security_verification;

#[test]
fn remem_backed_conditions_require_full_evidence() {
    for condition in [
        "remem_preloaded",
        "remem_seeded_sessionstart",
        "remem_e2e",
        "remem_oracle_retrieval",
        "remem_no_enrichment",
        "remem_fts_only",
    ] {
        assert!(super::verify::coding::requires_remem_evidence(condition));
    }
    assert!(!super::verify::coding::requires_remem_evidence("remem"));
    assert!(!super::verify::coding::requires_remem_evidence("no_memory"));
}

#[test]
fn public_condition_allowlist_matches_machine_registry() -> serde_json::Result<()> {
    let registry: Value =
        serde_json::from_str(include_str!("../../../eval/coding-bench/conditions.json"))?;
    let mut registered = registry["primary_conditions"]
        .as_array()
        .unwrap()
        .iter()
        .chain(registry["diagnostic_conditions"].as_array().unwrap())
        .map(|condition| condition["id"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    registered.insert("remem_preloaded".to_string());
    let verifier_conditions = super::verify::coding::public_coding_conditions()
        .iter()
        .map(|condition| (*condition).to_string())
        .collect::<BTreeSet<_>>();

    assert_eq!(verifier_conditions, registered);
    Ok(())
}

#[test]
fn claim_task_allowlist_matches_public_fixture() -> serde_json::Result<()> {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../eval/coding-bench/fixtures/tasks.json"
    ))?;
    let fixture_tasks = fixture["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    let claim_tasks = super::report::matrix::CLAIM_BEARING_TASK_IDS
        .into_iter()
        .collect::<BTreeSet<_>>();

    assert_eq!(claim_tasks, fixture_tasks);
    Ok(())
}

#[test]
fn claim_gate_excludes_historical_and_diagnostic_coding_conditions() {
    let historical = BTreeSet::from([
        "no_memory".to_string(),
        "remem_preloaded".to_string(),
        "curated_file_expert".to_string(),
    ]);
    assert!(!super::report::matrix::has_claim_bearing_coding_conditions(
        &historical
    ));

    let claim_bearing = BTreeSet::from([
        "no_memory".to_string(),
        "remem_e2e".to_string(),
        "curated_file_budgeted".to_string(),
    ]);
    assert!(super::report::matrix::has_claim_bearing_coding_conditions(
        &claim_bearing
    ));

    let mut mixed = claim_bearing;
    mixed.insert("oracle_evidence".to_string());
    assert!(!super::report::matrix::has_claim_bearing_coding_conditions(
        &mixed
    ));
}

#[test]
fn claim_gate_requires_verified_unique_same_task_matrix() {
    let valid = claim_matrix(&super::report::matrix::CLAIM_BEARING_TASK_IDS);
    assert!(super::report::matrix::has_claim_ready_coding_matrix(
        true, &valid
    ));
    assert!(!super::report::matrix::has_claim_ready_coding_matrix(
        false, &valid
    ));

    let mut smoke_identity = valid.clone();
    for outcome in &mut smoke_identity {
        outcome.run_phase = "smoke".to_string();
        outcome.matrix_namespace = "issue385-v1/smoke-reviewed".to_string();
    }
    assert!(!super::report::matrix::has_claim_ready_coding_matrix(
        true,
        &smoke_identity
    ));

    let mut mismatched = valid.clone();
    mismatched.retain(|run| {
        !(run.condition == "curated_file_budgeted"
            && run.task_id == super::report::matrix::CLAIM_BEARING_TASK_IDS[0])
    });
    assert!(!super::report::matrix::has_claim_ready_coding_matrix(
        true,
        &mismatched
    ));

    let duplicate_indices = ["no_memory", "remem_e2e", "curated_file_budgeted"]
        .into_iter()
        .flat_map(|condition| {
            super::report::matrix::CLAIM_BEARING_TASK_IDS
                .into_iter()
                .flat_map(move |task| {
                    (0..3).map(move |_| coding_outcome("matrix.json", condition, task, 0))
                })
        })
        .collect::<Vec<_>>();
    assert!(!super::report::matrix::has_claim_ready_coding_matrix(
        true,
        &duplicate_indices
    ));

    let mut extra_run = valid;
    extra_run.extend(
        ["no_memory", "remem_e2e", "curated_file_budgeted"]
            .into_iter()
            .map(|condition| {
                coding_outcome(
                    "matrix.json",
                    condition,
                    super::report::matrix::CLAIM_BEARING_TASK_IDS[0],
                    3,
                )
            }),
    );
    assert!(!super::report::matrix::has_claim_ready_coding_matrix(
        true, &extra_run
    ));

    let mut mixed_conditions = claim_matrix(&super::report::matrix::CLAIM_BEARING_TASK_IDS);
    mixed_conditions.push(coding_outcome(
        "matrix.json",
        "oracle_evidence",
        super::report::matrix::CLAIM_BEARING_TASK_IDS[0],
        0,
    ));
    assert!(!super::report::matrix::has_claim_ready_coding_matrix(
        true,
        &mixed_conditions
    ));
}

#[test]
fn committed_public_fixture_passes() -> Result<()> {
    let report = verify_benchmark_artifacts(BenchVerifyOptions {
        root: PathBuf::from("eval/public"),
    })?;

    assert!(report.passed, "{:#?}", report.failures);
    assert_eq!(report.manifests_checked, 6);
    assert_eq!(report.reports_checked, 6);
    assert_eq!(report.run_artifacts_checked, 65);
    assert_eq!(report.artifact_files_checked, 365);
    Ok(())
}

#[test]
fn verifier_rejects_report_version_that_differs_from_manifest() -> Result<()> {
    let root = copy_public_fixture("manifest-report-version-mismatch")?;
    mutate_json(
        &root.join("coding/manifests/issue385-smoke-v1.json"),
        |json| json["version"] = Value::String("mismatched-v2".to_string()),
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert_eq!(
        report.failures,
        vec![super::types::BenchVerifyFailure {
            path: "coding/reports/coding-report-v1.json".to_string(),
            message:
                "report benchmark_version \"v1\" must match manifest version \"mismatched-v2\""
                    .to_string(),
        }]
    );
    Ok(())
}

#[test]
fn verifier_rejects_memory_run_version_that_differs_from_report() -> Result<()> {
    let root = copy_public_fixture("memory-run-report-version-mismatch")?;
    mutate_json(
        &root.join("memory/artifacts/smoke-memory-001/run.json"),
        |json| json["benchmark_version"] = Value::String("mismatched-v2".to_string()),
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert_eq!(
        report.failures,
        vec![super::types::BenchVerifyFailure {
            path: "memory/artifacts/smoke-memory-001/run.json".to_string(),
            message: "memory run benchmark_version \"mismatched-v2\" must match report benchmark_version \"v1\""
                .to_string(),
        }]
    );
    Ok(())
}

#[test]
fn verifier_rejects_memory_run_benchmark_id_that_differs_from_report() -> Result<()> {
    let root = copy_public_fixture("memory-run-report-benchmark-id-mismatch")?;
    mutate_json(
        &root.join("memory/artifacts/smoke-memory-001/run.json"),
        |json| json["benchmark_id"] = Value::String("misrouted-benchmark".to_string()),
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert_eq!(
        report.failures,
        vec![super::types::BenchVerifyFailure {
            path: "memory/artifacts/smoke-memory-001/run.json".to_string(),
            message: "memory run benchmark_id \"misrouted-benchmark\" must match report benchmark_id \"remem-code-memory-smoke\""
                .to_string(),
        }]
    );
    Ok(())
}

#[test]
fn verifier_rejects_memory_run_suite_that_differs_from_report() -> Result<()> {
    let root = copy_public_fixture("memory-run-report-suite-mismatch")?;
    mutate_json(
        &root.join("memory/artifacts/smoke-memory-001/run.json"),
        |json| json["suite"] = Value::String("misrouted-suite".to_string()),
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert_eq!(
        report.failures,
        vec![super::types::BenchVerifyFailure {
            path: "memory/artifacts/smoke-memory-001/run.json".to_string(),
            message: "memory run suite \"misrouted-suite\" must match report suite \"remem-code-memory-smoke\""
                .to_string(),
        }]
    );
    Ok(())
}

#[test]
fn public_baseline_report_summarizes_committed_artifacts() -> Result<()> {
    let report = super::generate_public_baseline_report(Path::new("eval/public"))?;

    assert!(report.artifact_verifier.passed);
    assert_eq!(report.summary.manifest_count, 6);
    assert_eq!(report.summary.report_count, 6);
    assert_eq!(report.summary.run_artifact_count, 65);
    assert_eq!(report.summary.memory_system.run_artifact_count, 64);
    assert_eq!(report.summary.coding_agent.run_artifact_count, 1);
    assert_eq!(
        report.claim_gate.coding_outcome_stop_loss_status,
        "not_evaluated_insufficient_coding_matrix"
    );
    assert!(report
        .coding_condition_variance
        .iter()
        .any(|entry| entry.variance_status == "insufficient_runs_for_variance"));
    assert_eq!(report.coding_paired_statistics.len(), 2);
    assert!(report
        .coding_paired_statistics
        .iter()
        .all(|entry| entry.status == "insufficient"));
    Ok(())
}

#[test]
fn public_baseline_markdown_is_directional_and_separates_layers() -> Result<()> {
    let report = super::generate_public_baseline_report(Path::new("eval/public"))?;
    let markdown = super::render_public_baseline_markdown(&report);

    assert!(markdown.contains("directional_only_no_public_claim"));
    assert!(markdown.contains("## Memory-System Capability"));
    assert!(markdown.contains("## Coding-Agent Outcome"));
    assert!(markdown.contains("## Paired Coding Statistics"));
    assert!(markdown.contains("insufficient_runs_for_variance"));
    assert!(markdown.contains("insufficient"));
    assert!(markdown.contains("requires one verified issue385-v1/official-v1 report"));
    assert!(markdown.contains("must not be used for coding-task superiority claims"));
    Ok(())
}

#[test]
fn paired_statistics_compute_registered_task_cluster_effects() {
    let mut outcomes = claim_matrix(&super::report::matrix::CLAIM_BEARING_TASK_IDS);
    let first_task = super::report::matrix::CLAIM_BEARING_TASK_IDS[0];
    let second_task = super::report::matrix::CLAIM_BEARING_TASK_IDS[1];
    for outcome in &mut outcomes {
        outcome.resolved = match outcome.condition.as_str() {
            "remem_e2e" => {
                outcome.task_id == first_task
                    || (outcome.task_id == second_task && outcome.run_index == 0)
            }
            "curated_file_budgeted" => outcome.task_id == first_task && outcome.run_index == 0,
            "no_memory" => false,
            other => panic!("unexpected condition {other}"),
        };
    }

    let stats = super::report::coding_paired_statistics(&outcomes, true);
    let no_memory = stats
        .iter()
        .find(|entry| entry.comparison_id == "remem-e2e-vs-no-memory-v1")
        .unwrap();
    assert_eq!(no_memory.status, "computed");
    assert_eq!(no_memory.report_path.as_deref(), Some("matrix.json"));
    assert_eq!(no_memory.tasks, 16);
    assert_eq!(no_memory.runs_per_task, 3);
    assert_close(no_memory.treatment_resolved_rate.unwrap(), 1.0 / 12.0);
    assert_close(no_memory.control_resolved_rate.unwrap(), 0.0);
    assert_close(no_memory.effect_pp.unwrap(), 100.0 / 12.0);
    assert!(no_memory.ci_lower_pp.is_some());
    assert!(no_memory.ci_upper_pp.is_some());

    let curated = stats
        .iter()
        .find(|entry| entry.comparison_id == "remem-e2e-vs-curated-file-budgeted-v1")
        .unwrap();
    assert_close(curated.control_resolved_rate.unwrap(), 1.0 / 48.0);
    assert_close(curated.effect_pp.unwrap(), 6.25);
}

#[test]
fn paired_statistics_require_artifact_verification() {
    let outcomes = claim_matrix(&super::report::matrix::CLAIM_BEARING_TASK_IDS);

    let stats = super::report::coding_paired_statistics(&outcomes, false);

    assert_eq!(stats.len(), 2);
    assert!(stats.iter().all(|stat| stat.status == "insufficient"));
    assert!(stats.iter().all(|stat| stat.effect_pp.is_none()));
    assert!(stats.iter().all(|stat| {
        stat.insufficient_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("artifact verifier did not pass"))
    }));
}

#[test]
fn paired_statistics_reject_pre_target_failures() {
    let mut outcomes = claim_matrix(&super::report::matrix::CLAIM_BEARING_TASK_IDS);
    outcomes[0].target_started = Some(false);
    outcomes[0].resolved = false;
    outcomes[0].failure_reason = Some("compile_failure".to_string());

    let stats = super::report::coding_paired_statistics(&outcomes, true);

    assert_eq!(stats.len(), 2);
    assert!(stats.iter().all(|stat| stat.status == "insufficient"));
    assert!(stats.iter().all(|stat| stat.effect_pp.is_none()));
    assert!(stats.iter().all(|stat| {
        stat.insufficient_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("pre-target failures"))
    }));
    assert!(!super::report::matrix::has_claim_ready_coding_matrix(
        true, &outcomes
    ));
}

#[test]
fn verifier_rejects_missing_coding_test_log() -> Result<()> {
    let root = copy_public_fixture("missing-test-log")?;
    fs::remove_file(root.join("coding/artifacts/smoke-coding-001/test.log"))?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("artifact file for test_log is missing"));
    Ok(())
}

#[test]
fn coding_bench_attribution_verifier_rejects_unknown_coding_failure_reason() -> Result<()> {
    let root = copy_public_fixture("unknown-failure-reason")?;
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["resolved"] = Value::Bool(false);
            json["failure_reason"] = Value::String("free_text_failure".to_string());
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("unknown failure_reason enum"));
    Ok(())
}

#[test]
fn verifier_rejects_legacy_bare_coding_condition_identity() -> Result<()> {
    let root = copy_public_fixture("legacy-bare-condition")?;
    for path in [
        "coding/manifests/issue385-smoke-v1.json",
        "coding/reports/coding-report-v1.json",
    ] {
        mutate_json(&root.join(path), |json| {
            json["conditions"] = Value::Array(vec![Value::String("remem".to_string())]);
        })?;
    }
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| json["condition"] = Value::String("remem".to_string()),
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("unknown coding condition identity remem"));
    Ok(())
}

#[test]
fn verifier_rejects_coding_condition_layer_mismatch() -> Result<()> {
    let root = copy_public_fixture("condition-layer-mismatch")?;
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| json["condition"] = Value::String("no_memory".to_string()),
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("condition must be declared by its report"));
    Ok(())
}

#[test]
fn verifier_rejects_duplicate_coding_run_artifact_reference() -> Result<()> {
    let root = copy_public_fixture("duplicate-coding-run")?;
    mutate_json(&root.join("coding/reports/coding-report-v1.json"), |json| {
        let run_path = json["run_artifacts"][0].clone();
        json["run_artifacts"].as_array_mut().unwrap().push(run_path);
    })?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("run artifact path must be referenced only once"));
    Ok(())
}

#[test]
fn verifier_rejects_declared_coding_condition_without_run() -> Result<()> {
    let root = copy_public_fixture("unrepresented-coding-condition")?;
    for path in [
        "coding/manifests/issue385-smoke-v1.json",
        "coding/reports/coding-report-v1.json",
    ] {
        mutate_json(&root.join(path), |json| {
            json["conditions"] = Value::Array(vec![
                Value::String("remem_preloaded".to_string()),
                Value::String("no_memory".to_string()),
            ]);
        })?;
    }

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report)
        .contains("report conditions must exactly match conditions represented by run artifacts"));
    Ok(())
}

#[test]
fn verifier_accepts_not_applicable_context_audit_for_control() -> Result<()> {
    let root = copy_public_fixture("control-not-applicable")?;
    set_public_coding_condition(&root, "no_memory")?;
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["memory_contract"] = Value::Null;
            json["context_audit_status"] = Value::String("not_applicable".to_string());
            json["context_audit_failure_reason"] = Value::Null;
            json["remem_context_audit"] = Value::Null;
            json["injected_context_sha256"] = Value::Null;
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(report.passed, "{:#?}", report.failures);
    Ok(())
}

#[test]
fn verifier_rejects_omitted_control_context_audit_nulls() -> Result<()> {
    let root = copy_public_fixture("control-omitted-audit-nulls")?;
    set_public_coding_condition(&root, "no_memory")?;
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["memory_contract"] = Value::Null;
            json["context_audit_status"] = Value::String("not_applicable".to_string());
            json["injected_context_sha256"] = Value::Null;
            json.as_object_mut()
                .unwrap()
                .remove("context_audit_failure_reason");
            json.as_object_mut().unwrap().remove("remem_context_audit");
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    let text = failure_text(&report);
    assert!(text.contains("must include explicit context_audit_failure_reason"));
    assert!(text.contains("must include explicit remem_context_audit"));
    Ok(())
}

#[test]
fn context_audit_snapshot_rejects_unknown_fields() -> serde_json::Result<()> {
    let mut snapshot = serde_json::to_value(serde_json::from_str::<
        crate::eval::coding_bench::RememContextAuditSnapshot,
    >(
        r#"{
                "injection_run_id":"run-1",
                "bundle_schema_version":1,
                "plan_schema_version":1,
                "policy_version":"policy-v1",
                "relevance_policy_version":"relevance-v1",
                "plan_hash":"plan-hash",
                "audit_hash":"audit-hash",
                "injection_binding_hash":"binding-hash",
                "degraded_mode":"full",
                "candidates_considered":1,
                "selected_count":1,
                "dropped_count":0,
                "token_budget":100,
                "token_estimate":10,
                "truncation_reason":null,
                "canonical_audit_json":"{}"
            }"#,
    )?)?;
    snapshot["unexpected"] = Value::Bool(true);

    assert!(
        serde_json::from_value::<crate::eval::coding_bench::RememContextAuditSnapshot>(snapshot)
            .is_err()
    );
    Ok(())
}

#[test]
fn verifier_rejects_memory_contract_for_control() -> Result<()> {
    let root = copy_public_fixture("control-memory-contract")?;
    set_public_coding_condition(&root, "no_memory")?;
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["context_audit_status"] = Value::String("not_applicable".to_string());
            json["context_audit_failure_reason"] = Value::Null;
            json["remem_context_audit"] = Value::Null;
            json["injected_context_sha256"] = Value::Null;
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("must not include memory_contract"));
    Ok(())
}

#[test]
fn verifier_requires_context_audit_for_current_remem_condition() -> Result<()> {
    let root = copy_public_fixture("missing-current-context-audit")?;
    for path in [
        "coding/manifests/issue385-smoke-v1.json",
        "coding/reports/coding-report-v1.json",
    ] {
        mutate_json(&root.join(path), |json| {
            json["conditions"] =
                Value::Array(vec![Value::String("remem_seeded_sessionstart".to_string())]);
        })?;
    }
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["condition"] = Value::String("remem_seeded_sessionstart".to_string());
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    let text = failure_text(&report);
    assert!(text.contains("must carry verified ContextAudit status"));
    assert!(text.contains("must include a ContextAudit snapshot"));
    Ok(())
}

#[test]
fn coding_bench_attribution_verifier_rejects_invalid_memory_contract() -> Result<()> {
    let root = copy_public_fixture("invalid-memory-contract")?;
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["resolved"] = Value::Bool(false);
            json["failure_reason"] = Value::String("stale_memory_followed".to_string());
            json["memory_contract"]["citation_precision"] = Value::from(1.5);
            json["memory_contract"]["memory_hurt"] = Value::Bool(false);
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    let text = failure_text(&report);
    assert!(text.contains("memory_contract.citation_precision"));
    assert!(text.contains("memory_contract.memory_hurt=true"));
    Ok(())
}

#[test]
fn verifier_rejects_missing_memory_supporting_ids() -> Result<()> {
    let root = copy_public_fixture("missing-memory-support")?;
    mutate_json(
        &root.join("memory/artifacts/smoke-memory-001/run.json"),
        |json| {
            json["retrieval"]["gold_supporting_event_ids"] = Value::Array(Vec::new());
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("missing gold supporting evidence IDs"));
    Ok(())
}

#[test]
fn verifier_rejects_private_remem_data_path() -> Result<()> {
    let root = copy_public_fixture("private-remem-path")?;
    mutate_json(
        &root.join("memory/artifacts/smoke-memory-001/run.json"),
        |json| {
            json["environment"]["remem_data_dir"] = Value::String("~/.remem".to_string());
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    let text = failure_text(&report);
    assert!(text.contains("private remem path"));
    assert!(text.contains("temporary isolation"));
    Ok(())
}

pub(super) fn failure_text(report: &super::types::BenchVerifyReport) -> String {
    report
        .failures
        .iter()
        .map(|failure| format!("{}: {}", failure.path, failure.message))
        .collect::<Vec<_>>()
        .join("\n")
}

fn claim_matrix(tasks: &[&str]) -> Vec<super::report::CodingTaskOutcome> {
    ["no_memory", "remem_e2e", "curated_file_budgeted"]
        .into_iter()
        .flat_map(|condition| {
            tasks.iter().flat_map(move |task| {
                (0..3)
                    .map(move |run_index| coding_outcome("matrix.json", condition, task, run_index))
            })
        })
        .collect()
}

fn coding_outcome(
    report_path: &str,
    condition: &str,
    task_id: &str,
    run_index: u32,
) -> super::report::CodingTaskOutcome {
    super::report::CodingTaskOutcome {
        report_path: report_path.to_string(),
        benchmark_id: "issue385-v1".to_string(),
        benchmark_version: "official-v1".to_string(),
        run_phase: "official".to_string(),
        matrix_namespace: "issue385-v1/official-v1".to_string(),
        condition: condition.to_string(),
        task_id: task_id.to_string(),
        run_index,
        attempt_id: Some(format!("attempt-{condition}-{task_id}-{run_index}")),
        target_started: Some(true),
        resolved: true,
        failure_reason: None,
        tokens_total: Some(1),
        turns: Some(1),
        wall_time_ms: Some(1),
        memory_helped: None,
        memory_hurt: None,
    }
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {actual} to be close to {expected}"
    );
}

fn set_public_coding_condition(root: &Path, condition: &str) -> Result<()> {
    for path in [
        "coding/manifests/issue385-smoke-v1.json",
        "coding/reports/coding-report-v1.json",
    ] {
        mutate_json(&root.join(path), |json| {
            json["conditions"] = Value::Array(vec![Value::String(condition.to_string())]);
        })?;
    }
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| json["condition"] = Value::String(condition.to_string()),
    )
}

pub(super) fn mutate_json(path: &Path, mutate: impl FnOnce(&mut Value)) -> Result<()> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut json: Value =
        serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
    mutate(&mut json);
    fs::write(path, serde_json::to_string_pretty(&json)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub(super) fn copy_public_fixture(label: &str) -> Result<PathBuf> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let root = std::env::temp_dir().join(format!("remem-bench-artifact-{label}-{millis}"));
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    copy_dir_all(Path::new("eval/public"), &root)?;
    Ok(root)
}

fn copy_dir_all(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).with_context(|| format!("create {}", to.display()))?;
    for entry in fs::read_dir(from).with_context(|| format!("read {}", from.display()))? {
        let entry = entry?;
        let from_path = entry.path();
        let to_path = to.join(entry.file_name());
        if from_path.is_dir() {
            copy_dir_all(&from_path, &to_path)?;
        } else {
            fs::copy(&from_path, &to_path).with_context(|| {
                format!("copy {} to {}", from_path.display(), to_path.display())
            })?;
        }
    }
    Ok(())
}
