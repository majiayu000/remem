use std::collections::BTreeMap;
use std::fs;

use anyhow::Result;
use serde_json::Value;

use super::{copy_public_fixture, mutate_json};
use crate::eval::bench_artifact::authority::gh931::meets_non_inferiority_margin;
use crate::eval::bench_artifact::types::{
    AuthorityStatus, BenchmarkLayer, ClaimRegistryGate, ClaimRegistryPolicy, CodingMemoryContract,
    CodingRunArtifact, CodingRunMetrics, CuratorBudget, CuratorLogArtifact, CuratorSession,
    CuratorTotals, Gh931AuthorityVerdict, ImplementationAuthorityBinding,
    OfficialCodingCommandResult, OfficialCodingMaintenanceEvidence,
    OfficialCodingMaintenanceMeasurement, OfficialCodingTestEvidence, PublicBenchmarkReport,
    ReportVerifierMetadata, RunEnvironment, VerifiedArtifact, VerifiedBenchmarkArtifacts,
};
use crate::eval::bench_artifact::{verify_benchmark_artifacts, BenchVerifyOptions};

#[test]
fn tampered_registry_pass_cannot_authorize_smoke_evidence() -> Result<()> {
    let root = copy_public_fixture("gh931-registry-pass-smoke")?;
    let registry_path = root.join("claim-registry.json");
    fs::copy("eval/claims/registry.json", &registry_path)?;
    mutate_json(&registry_path, |registry| {
        registry["locked"] = Value::Bool(true);
        for claim in registry["claims"].as_array_mut().unwrap() {
            claim["status"] = Value::String("PASS".to_string());
        }
    })?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions {
        root,
        claim_registry_path: registry_path,
    })?;

    assert!(report.passed, "{:#?}", report.failures);
    assert_eq!(
        report.authority_verdict.gh931.status,
        AuthorityStatus::Insufficient
    );
    assert!(report
        .authority_verdict
        .gh931
        .claims
        .iter()
        .all(|claim| claim.status != AuthorityStatus::Pass));
    assert_eq!(
        report.authority_verdict.gh931.registry.declared_statuses,
        vec![AuthorityStatus::Pass; 3]
    );
    assert!(report
        .authority_verdict
        .consumed_bytes
        .contains_key("claim-registry.json"));
    Ok(())
}

#[test]
fn locked_registry_rejects_relaxed_numeric_claim_gates() -> Result<()> {
    for mutation in [
        "superiority-effect",
        "superiority-ci",
        "non-inferiority-margin",
        "maintenance-reduction",
        "memory-hurt",
        "stale-memory",
        "allowed-wording",
        "forbidden-wording",
    ] {
        let mut verified = complete_verified_matrix(AuthorityStatus::Insufficient)?;
        let policy = &mut verified.claim_registry.as_mut().unwrap().value;
        match mutation {
            "superiority-effect" => {
                let ClaimRegistryGate::Superiority(gate) = &mut policy.claims[0].gate else {
                    panic!("superiority gate")
                };
                gate.min_effect_pp = -100.0;
            }
            "superiority-ci" => {
                let ClaimRegistryGate::Superiority(gate) = &mut policy.claims[0].gate else {
                    panic!("superiority gate")
                };
                gate.ci_lower_bound_pp_gt = -100.0;
            }
            "non-inferiority-margin" => {
                let ClaimRegistryGate::NonInferiority(gate) = &mut policy.claims[1].gate else {
                    panic!("non-inferiority gate")
                };
                gate.non_inferiority_margin_pp = 100.0;
            }
            "maintenance-reduction" => {
                let ClaimRegistryGate::NonInferiority(gate) = &mut policy.claims[1].gate else {
                    panic!("non-inferiority gate")
                };
                gate.human_maintenance_reduction_min_pct = -100.0;
            }
            "memory-hurt" => {
                let ClaimRegistryGate::StopLoss(gate) = &mut policy.claims[2].gate else {
                    panic!("stop-loss gate")
                };
                gate.memory_hurt_max_pct = 100.0;
            }
            "stale-memory" => {
                let ClaimRegistryGate::StopLoss(gate) = &mut policy.claims[2].gate else {
                    panic!("stop-loss gate")
                };
                gate.stale_memory_followed_max_pct = 100.0;
            }
            "allowed-wording" => {
                policy.claims[0].allowed_wording = vec!["remem beats everything".to_string()];
            }
            "forbidden-wording" => {
                policy.claims[0].forbidden_wording = vec!["unrelated phrase".to_string()];
            }
            _ => unreachable!(),
        }

        let verdict = evaluate_gh931(&verified, &[]);
        assert!(!verdict.registry.policy_valid, "{mutation}");
        assert_ne!(verdict.status, AuthorityStatus::Pass, "{mutation}");
    }
    Ok(())
}

#[test]
fn complete_exact_matrix_reuses_registered_paired_statistics() -> Result<()> {
    let verified = complete_verified_matrix(AuthorityStatus::Insufficient)?;

    let verdict = evaluate_gh931(&verified, &[]);

    assert!(verdict.completeness.complete);
    assert!(verdict.completeness.attempts_ready);
    assert_eq!(verdict.completeness.observed_runs, 144);
    assert_eq!(verdict.paired_statistics.len(), 2);
    assert!(verdict.paired_statistics.iter().all(|statistic| {
        statistic.status == "computed"
            && statistic.algorithm == "task_cluster_paired_bootstrap_v1"
            && statistic.tasks == 16
            && statistic.runs_per_task == 3
    }));
    for claim in &verdict.claims {
        let policy = verified
            .claim_registry
            .as_ref()
            .expect("claim registry")
            .value
            .claims
            .iter()
            .find(|policy| policy.id == claim.id)
            .expect("matching claim policy");
        assert_eq!(claim.allowed_wording, policy.allowed_wording);
        assert_eq!(claim.forbidden_wording, policy.forbidden_wording);
    }
    let report = verdict.report.unwrap();
    assert_eq!(report.path, "coding/reports/issue385-official-v1.json");
    assert_eq!(report.sha256, "a".repeat(64));
    assert_eq!(report.models_by_condition.len(), 3);
    assert_eq!(report.platforms, vec!["linux/x86_64"]);
    assert_eq!(report.producing_shas, vec!["c".repeat(40)]);
    assert_eq!(report.production_input_trees, vec!["e".repeat(64)]);
    assert_eq!(report.source_dirty_attestations, vec![Some(false)]);
    Ok(())
}

#[test]
fn unsigned_official_evidence_cannot_authorize_gh931() -> Result<()> {
    let mut verified = complete_verified_matrix(AuthorityStatus::Pass)?;
    verified.official_evidence_authenticated = false;
    attach_curator_evidence(&mut verified);
    attach_treatment_evidence(&mut verified, 0.0, 1);

    let verdict = evaluate_gh931(&verified, &[]);

    assert_eq!(verdict.status, AuthorityStatus::Insufficient);
    assert_eq!(verdict.stop_loss.status, AuthorityStatus::Insufficient);
    assert_eq!(verdict.maintenance.status, AuthorityStatus::Insufficient);
    assert!(verdict
        .diagnostics
        .iter()
        .any(|message| message.contains("governed scorer and supervisor receipts")));
    Ok(())
}

#[test]
fn condition_completion_keeps_remem_e2e_separate_and_excludes_pre_target_runs() -> Result<()> {
    let mut verified = complete_verified_matrix(AuthorityStatus::Insufficient)?;
    for run in &mut verified.coding_runs {
        if run.value.condition == "no_memory" {
            run.value.resolved = true;
        }
    }
    let excluded = verified
        .coding_runs
        .iter_mut()
        .find(|run| run.value.condition == "remem_e2e")
        .expect("remem_e2e run");
    excluded.value.target_started = Some(false);
    excluded.value.resolved = true;

    let verdict = evaluate_gh931(&verified, &[]);
    let remem = verdict
        .condition_completion
        .iter()
        .find(|completion| completion.condition == "remem_e2e")
        .expect("remem_e2e completion population");

    assert_eq!(remem.eligible_started, 47);
    assert_eq!(remem.resolved, 47);
    assert_eq!(
        verdict
            .condition_completion
            .iter()
            .find(|completion| completion.condition == "no_memory")
            .expect("no_memory completion population")
            .eligible_started,
        48
    );
    Ok(())
}

#[test]
fn pre_target_or_duplicate_attempt_makes_matrix_insufficient() -> Result<()> {
    let mut pre_target = complete_verified_matrix(AuthorityStatus::Pass)?;
    pre_target.coding_runs[0].value.target_started = Some(false);
    let verdict = evaluate_gh931(&pre_target, &[]);
    assert_eq!(verdict.status, AuthorityStatus::Insufficient);
    assert!(verdict.completeness.complete);
    assert!(!verdict.completeness.attempts_ready);
    assert!(verdict
        .paired_statistics
        .iter()
        .all(|statistic| statistic.status == "insufficient"));

    let mut duplicate = complete_verified_matrix(AuthorityStatus::Pass)?;
    duplicate.coding_runs[1].value.attempt_id = duplicate.coding_runs[0].value.attempt_id.clone();
    let verdict = evaluate_gh931(&duplicate, &[]);
    assert_eq!(verdict.status, AuthorityStatus::Insufficient);
    assert!(verdict.completeness.complete);
    assert!(!verdict.completeness.attempts_ready);
    Ok(())
}

#[test]
fn memory_harm_or_stale_followed_breach_fails_stop_loss() -> Result<()> {
    let mut verified = complete_verified_matrix(AuthorityStatus::Pass)?;
    let remem_indices = verified
        .coding_runs
        .iter()
        .enumerate()
        .filter(|(_, run)| run.value.condition == "remem_e2e")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    verified.coding_runs[remem_indices[0]]
        .value
        .memory_contract
        .as_mut()
        .unwrap()
        .memory_hurt = true;

    let declaration_only = evaluate_gh931(&verified, &[]);
    assert_eq!(declaration_only.stop_loss.memory_hurt_rate_pct, Some(0.0));
    assert_eq!(declaration_only.stop_loss.status, AuthorityStatus::Pass);

    verified.coding_runs[remem_indices[0]]
        .value
        .memory_contract
        .as_mut()
        .unwrap()
        .stale_used_count = 1;
    verified.coding_runs[remem_indices[0]]
        .value
        .memory_contract
        .as_mut()
        .unwrap()
        .memory_hurt = false;
    verified.coding_runs[remem_indices[1]].value.resolved = false;
    verified.coding_runs[remem_indices[1]].value.failure_reason =
        Some("stale_memory_followed".to_string());

    let verdict = evaluate_gh931(&verified, &[]);

    assert_eq!(verdict.status, AuthorityStatus::Fail);
    assert_eq!(verdict.stop_loss.status, AuthorityStatus::Fail);
    assert!(verdict.stop_loss.memory_hurt_rate_pct.unwrap() > 2.0);
    assert!(verdict.stop_loss.stale_memory_followed_rate_pct.unwrap() > 1.0);
    Ok(())
}

#[test]
fn verifier_rejects_declared_memory_hurt_that_disagrees_with_raw_attribution() -> Result<()> {
    let root = copy_public_fixture("gh931-memory-hurt-mismatch")?;
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |run| {
            run["memory_contract"]["stale_used_count"] = Value::from(1);
            run["memory_contract"]["memory_hurt"] = Value::Bool(false);
        },
    )?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(report.failures.iter().any(|failure| failure
        .message
        .contains("memory_contract.memory_hurt must match recomputed memory harm")));
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

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    let text = super::failure_text(&report);
    assert!(text.contains("memory_contract.citation_precision"));
    assert!(text.contains("memory_contract.memory_hurt must match recomputed memory harm"));
    Ok(())
}

#[test]
fn mixed_or_malformed_official_provenance_is_insufficient() -> Result<()> {
    let mut genuine = complete_verified_matrix(AuthorityStatus::Pass)?;
    attach_curator_evidence(&mut genuine);
    attach_treatment_evidence(&mut genuine, 0.0, 1);
    assert_eq!(evaluate_gh931(&genuine, &[]).status, AuthorityStatus::Pass);

    let mut malformed_sha = genuine.clone();
    malformed_sha.coding_runs[0].value.environment.remem_commit = "not-a-sha".to_string();
    assert_eq!(
        evaluate_gh931(&malformed_sha, &[]).status,
        AuthorityStatus::Insufficient
    );

    let mut mixed_sha = genuine.clone();
    mixed_sha.coding_runs[0].value.environment.remem_commit = "f".repeat(40);
    assert_eq!(
        evaluate_gh931(&mixed_sha, &[]).status,
        AuthorityStatus::Pass
    );

    let mut malformed_tree = genuine.clone();
    malformed_tree.coding_runs[0]
        .value
        .environment
        .production_input_tree_sha256 = Some("not-a-tree".to_string());
    assert_eq!(
        evaluate_gh931(&malformed_tree, &[]).status,
        AuthorityStatus::Insufficient
    );

    let mut mixed_tree = genuine.clone();
    mixed_tree.coding_runs[0]
        .value
        .environment
        .production_input_tree_sha256 = Some("f".repeat(64));
    assert_eq!(
        evaluate_gh931(&mixed_tree, &[]).status,
        AuthorityStatus::Insufficient
    );

    let mut mixed_platform = genuine.clone();
    mixed_platform.coding_runs[0].value.environment.arch = "aarch64".to_string();
    assert_eq!(
        evaluate_gh931(&mixed_platform, &[]).status,
        AuthorityStatus::Insufficient
    );

    let mut dirty = genuine.clone();
    dirty.coding_runs[0].value.environment.source_dirty = Some(true);
    assert_eq!(
        evaluate_gh931(&dirty, &[]).status,
        AuthorityStatus::Insufficient
    );

    let mut incomplete_model = genuine;
    incomplete_model.coding_runs[0].value.model = serde_json::json!({"model": ""});
    assert_eq!(
        evaluate_gh931(&incomplete_model, &[]).status,
        AuthorityStatus::Insufficient
    );
    Ok(())
}

#[test]
fn exact_non_inferiority_ci_margin_is_inclusive() -> Result<()> {
    assert!(meets_non_inferiority_margin(-3.0, -3.0, 3.0));
    assert!(!meets_non_inferiority_margin(-3.0, -3.000_001, 3.0));
    Ok(())
}

#[test]
fn cross_condition_model_mismatch_is_insufficient() -> Result<()> {
    let mut verified = complete_verified_matrix(AuthorityStatus::Pass)?;
    attach_curator_evidence(&mut verified);
    attach_treatment_evidence(&mut verified, 0.0, 1);
    verified
        .coding_runs
        .iter_mut()
        .find(|run| run.value.condition == "curated_file_budgeted")
        .unwrap()
        .value
        .model = serde_json::json!({"provider": "fixture", "model": "different"});

    let verdict = evaluate_gh931(&verified, &[]);

    assert_eq!(verdict.status, AuthorityStatus::Insufficient);
    assert!(verdict
        .diagnostics
        .iter()
        .any(|message| message.contains("model identity")));
    Ok(())
}

#[test]
fn official_tree_must_match_current_runtime_implementation() -> Result<()> {
    let mut verified = complete_verified_matrix(AuthorityStatus::Pass)?;
    attach_curator_evidence(&mut verified);
    attach_treatment_evidence(&mut verified, 0.0, 1);

    let implementation = implementation_binding("f".repeat(64));
    let verdict =
        crate::eval::bench_artifact::authority::gh931::evaluate(&verified, &[], &implementation);

    assert_eq!(verdict.status, AuthorityStatus::Insufficient);
    assert!(verdict
        .diagnostics
        .iter()
        .any(|message| message.contains("current runtime implementation tree")));
    Ok(())
}

#[test]
fn smoke_curated_run_without_maintenance_artifacts_remains_directional() -> Result<()> {
    let root = copy_public_fixture("gh931-curated-smoke-directional")?;
    for path in [
        "coding/manifests/issue385-smoke-v1.json",
        "coding/reports/coding-report-v1.json",
    ] {
        mutate_json(&root.join(path), |value| {
            value["conditions"] = serde_json::json!(["curated_file_budgeted"]);
        })?;
    }
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |run| {
            run["condition"] = Value::String("curated_file_budgeted".to_string());
            run.as_object_mut().unwrap().remove("memory_contract");
            run["context_audit_status"] = Value::String("not_applicable".to_string());
            run["context_audit_failure_reason"] = Value::Null;
            run["remem_context_audit"] = Value::Null;
            run["injected_context_sha256"] = Value::Null;
        },
    )?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(report.passed, "{:#?}", report.failures);
    assert_eq!(
        report.authority_verdict.gh931.status,
        AuthorityStatus::Insufficient
    );
    Ok(())
}

#[test]
fn insufficient_registry_declaration_cannot_veto_computed_pass() -> Result<()> {
    let verified = complete_verified_matrix(AuthorityStatus::Insufficient)?;

    let verdict = evaluate_gh931(&verified, &[]);
    let claim = verdict
        .claims
        .iter()
        .find(|claim| claim.id == "remem-e2e-vs-no-memory-v1")
        .unwrap();

    assert_eq!(
        claim.declared_registry_status,
        AuthorityStatus::Insufficient
    );
    assert_eq!(claim.status, AuthorityStatus::Pass);
    Ok(())
}

#[test]
fn missing_treatment_maintenance_evidence_is_insufficient() -> Result<()> {
    let mut verified = complete_verified_matrix(AuthorityStatus::Insufficient)?;
    attach_curator_evidence(&mut verified);

    let verdict = evaluate_gh931(&verified, &[]);

    assert_eq!(verdict.status, AuthorityStatus::Insufficient);
    assert_eq!(verdict.maintenance.status, AuthorityStatus::Insufficient);
    assert_eq!(verdict.maintenance.remem_minutes_per_100_sessions, None);
    assert_eq!(verdict.maintenance.reduction_pct, None);
    assert!(verdict
        .maintenance
        .diagnostics
        .iter()
        .any(|message| message.contains("treatment")));
    Ok(())
}

#[test]
fn zero_and_measured_treatment_work_recompute_maintenance_reduction() -> Result<()> {
    let mut zero_work = complete_verified_matrix(AuthorityStatus::Insufficient)?;
    attach_curator_evidence(&mut zero_work);
    attach_treatment_evidence(&mut zero_work, 0.0, 3);
    let zero_verdict = evaluate_gh931(&zero_work, &[]);
    assert_eq!(zero_verdict.maintenance.status, AuthorityStatus::Pass);
    assert_eq!(zero_verdict.maintenance.remem_sessions, Some(144));
    assert_eq!(
        zero_verdict.maintenance.remem_minutes_per_100_sessions,
        Some(0.0)
    );
    assert_eq!(zero_verdict.maintenance.reduction_pct, Some(100.0));

    let mut measured = complete_verified_matrix(AuthorityStatus::Insufficient)?;
    attach_curator_evidence(&mut measured);
    attach_treatment_evidence(&mut measured, 0.5, 2);
    let measured_verdict = evaluate_gh931(&measured, &[]);
    assert_eq!(measured_verdict.maintenance.status, AuthorityStatus::Pass);
    assert_eq!(
        measured_verdict.maintenance.remem_minutes_per_100_sessions,
        Some(25.0)
    );
    assert_eq!(measured_verdict.maintenance.reduction_pct, Some(87.5));
    Ok(())
}

fn complete_verified_matrix(
    declared_status: AuthorityStatus,
) -> Result<VerifiedBenchmarkArtifacts> {
    let mut registry_json: Value = serde_json::from_slice(&fs::read("eval/claims/registry.json")?)?;
    registry_json["locked"] = Value::Bool(true);
    for claim in registry_json["claims"].as_array_mut().unwrap() {
        claim["status"] = serde_json::to_value(declared_status)?;
    }
    let policy: ClaimRegistryPolicy = serde_json::from_value(registry_json)?;
    let report_path = "coding/reports/issue385-official-v1.json".to_string();
    let mut run_paths = Vec::new();
    let mut coding_runs = Vec::new();
    for condition in ["no_memory", "remem_e2e", "curated_file_budgeted"] {
        for task_id in super::super::report::matrix::CLAIM_BEARING_TASK_IDS {
            for run_index in 0..3 {
                let path = format!("coding/artifacts/{condition}/{task_id}/{run_index}/run.json");
                run_paths.push(path.clone());
                coding_runs.push(VerifiedArtifact {
                    path,
                    sha256: format!("{run_index:064x}"),
                    value: coding_run(condition, task_id, run_index),
                });
            }
        }
    }
    let official_coding_tests = coding_runs
        .iter()
        .map(|run| {
            let value = &run.value;
            (
                run.path.clone(),
                VerifiedArtifact {
                    path: format!("{}/test.json", run.path.trim_end_matches("/run.json")),
                    sha256: "9".repeat(64),
                    value: OfficialCodingTestEvidence {
                        schema_version: 1,
                        task_id: value.task_id.clone(),
                        condition: value.condition.clone(),
                        run_index: value.run_index,
                        attempt_id: value.attempt_id.clone().unwrap(),
                        commands: vec![OfficialCodingCommandResult {
                            command: "cargo test --test target".to_string(),
                            exit_code: Some(if value.resolved { 0 } else { 1 }),
                            timed_out: false,
                        }],
                    },
                },
            )
        })
        .collect();
    Ok(VerifiedBenchmarkArtifacts {
        reports: vec![VerifiedArtifact {
            path: report_path,
            sha256: "a".repeat(64),
            value: PublicBenchmarkReport {
                schema_version: 1,
                benchmark_id: "issue385-v1".to_string(),
                benchmark_version: "official-v1".to_string(),
                suite: None,
                run_phase: Some("official".to_string()),
                matrix_namespace: Some("issue385-v1/official-v1".to_string()),
                layer: BenchmarkLayer::CodingAgentOutcome,
                conditions: vec![
                    "no_memory".to_string(),
                    "remem_e2e".to_string(),
                    "curated_file_budgeted".to_string(),
                ],
                schema_refs: Vec::new(),
                run_artifacts: run_paths,
                aggregate_metrics: Value::Null,
                claim_level: "official".to_string(),
                verifier: ReportVerifierMetadata {
                    required: true,
                    schema_version: 1,
                },
            },
        }],
        coding_runs,
        official_coding_tests,
        claim_registry: Some(VerifiedArtifact {
            path: "eval/claims/registry.json".to_string(),
            sha256: "b".repeat(64),
            value: policy,
        }),
        official_evidence_authenticated: true,
        ..VerifiedBenchmarkArtifacts::default()
    })
}

fn coding_run(condition: &str, task_id: &str, run_index: u32) -> CodingRunArtifact {
    CodingRunArtifact {
        schema_version: 1,
        benchmark_id: "issue385-v1".to_string(),
        benchmark_version: "official-v1".to_string(),
        run_phase: "official".to_string(),
        matrix_namespace: "issue385-v1/official-v1".to_string(),
        layer: BenchmarkLayer::CodingAgentOutcome,
        condition: condition.to_string(),
        task_id: task_id.to_string(),
        run_index,
        attempt_id: Some(format!("attempt-{condition}-{task_id}-{run_index}")),
        target_started: Some(true),
        model: serde_json::json!({"provider": "fixture", "model": "same-model"}),
        environment: RunEnvironment {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            remem_commit: "c".repeat(40),
            remem_data_dir: "temp://gh931-fixture".to_string(),
            docker_image_digest: None,
            fixture_revision: Some("issue385-v1".to_string()),
            repo_base_commit: Some("d".repeat(40)),
            source_dirty: Some(false),
            production_input_tree_sha256: Some("e".repeat(64)),
        },
        resolved: condition != "no_memory",
        failure_reason: (condition == "no_memory").then(|| "test_failure".to_string()),
        metrics: CodingRunMetrics {
            tokens_input: Some(1),
            tokens_output: Some(1),
            tokens_total: Some(2),
            turns: Some(1),
            wall_time_ms: Some(1),
            tool_calls: Some(1),
            commands_run: Some(1),
        },
        memory_contract: (condition == "remem_e2e").then(|| CodingMemoryContract {
            injected_memory_ids: vec![1],
            used_memory_ids: vec![1],
            citation_precision: 1.0,
            citation_recall: 1.0,
            stale_used_count: 0,
            irrelevant_injection_count: 0,
            missing_relevant_memory_count: 0,
            memory_helped: true,
            memory_hurt: false,
        }),
        context_audit_status: None,
        context_audit_failure_reason: None,
        remem_context_audit: None,
        injected_context_sha256: None,
        artifacts: BTreeMap::new(),
    }
}

fn attach_curator_evidence(verified: &mut VerifiedBenchmarkArtifacts) {
    for run in verified
        .coding_runs
        .iter()
        .filter(|run| run.value.condition == "curated_file_budgeted")
    {
        verified.curator_logs.insert(
            run.path.clone(),
            VerifiedArtifact {
                path: format!("curator/{}.json", run.value.task_id),
                sha256: "f".repeat(64),
                value: CuratorLogArtifact {
                    schema_version: 1,
                    condition: "curated_file_budgeted".to_string(),
                    task_id: run.value.task_id.clone(),
                    target_blind: true,
                    budget: CuratorBudget {
                        minutes_per_session: 3.0,
                        max_chars: 4_000,
                    },
                    sessions: vec![CuratorSession {
                        episode_id: format!("{}-history", run.value.task_id),
                        minutes_spent: 2.0,
                        edit_count: 1,
                        deletion_count: 0,
                        conflict_resolution_count: 0,
                        chars_after: 1,
                    }],
                    totals: CuratorTotals {
                        maintenance_minutes: 2.0,
                        update_count: 1,
                        deletion_count: 0,
                        conflict_resolution_count: 0,
                    },
                    final_char_count: 1,
                    final_file_sha256: "0".repeat(64),
                },
            },
        );
    }
}

fn attach_treatment_evidence(
    verified: &mut VerifiedBenchmarkArtifacts,
    minutes: f64,
    session_count: usize,
) {
    for run in verified
        .coding_runs
        .iter()
        .filter(|run| run.value.condition == "remem_e2e")
    {
        let measurement = if minutes == 0.0 {
            OfficialCodingMaintenanceMeasurement::ZeroWork {
                minutes,
                work_events: 0,
                session_count,
            }
        } else {
            OfficialCodingMaintenanceMeasurement::SupervisorTimed {
                minutes,
                session_count,
            }
        };
        verified.treatment_maintenance.insert(
            run.path.clone(),
            VerifiedArtifact {
                path: format!(
                    "{}/maintenance.json",
                    run.path.trim_end_matches("/run.json")
                ),
                sha256: "8".repeat(64),
                value: OfficialCodingMaintenanceEvidence {
                    schema_version: 1,
                    task_id: run.value.task_id.clone(),
                    condition: run.value.condition.clone(),
                    run_index: run.value.run_index,
                    attempt_id: run.value.attempt_id.clone().unwrap(),
                    measurement,
                },
            },
        );
    }
}

fn implementation_binding(tree: String) -> ImplementationAuthorityBinding {
    crate::eval::bench_artifact::authority::implementation::binding_from_parts(
        Some(&"1".repeat(40)),
        Some(&"1".repeat(40)),
        Some(false),
        Some(false),
        Some(&tree),
        Some(&tree),
        Some(&"2".repeat(64)),
    )
}

fn evaluate_gh931(
    verified: &VerifiedBenchmarkArtifacts,
    failures: &[crate::eval::bench_artifact::types::BenchVerifyFailure],
) -> Gh931AuthorityVerdict {
    crate::eval::bench_artifact::authority::gh931::evaluate(
        verified,
        failures,
        &implementation_binding("e".repeat(64)),
    )
}
