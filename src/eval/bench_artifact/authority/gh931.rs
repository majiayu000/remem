use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::super::report::{
    coding_paired_statistics, coding_report_structurally_complete, CodingPairedStatistic,
    CodingTaskOutcome,
};
use super::super::types::{
    AuthorityStatus, BenchVerifyFailure, ClaimRegistryClaimPolicy, ClaimRegistryGate,
    ClaimRegistryPolicy, ClaimStopLossGate, Gh931AuthorityVerdict, Gh931ClaimVerdict,
    Gh931Completeness, Gh931ConditionCompletion, Gh931MaintenanceVerdict, Gh931RegistryBinding,
    Gh931ReportBinding, Gh931StopLossVerdict, ImplementationAuthorityBinding, VerifiedArtifact,
    VerifiedBenchmarkArtifacts,
};

const EXPECTED_RUNS: usize = 16 * 3 * 3;
const NO_MEMORY_CLAIM: &str = "remem-e2e-vs-no-memory-v1";
const CURATED_CLAIM: &str = "remem-e2e-vs-curated-file-budgeted-v1";
const STOP_LOSS_CLAIM: &str = "remem-e2e-stop-loss-v1";
const REGISTERED_WORDING_SHA256: &str =
    "c62388db626812265580780c03c345c284163ab3b75c424000f7627a6dffc18b";

pub(in crate::eval::bench_artifact) fn evaluate(
    verified: &VerifiedBenchmarkArtifacts,
    verifier_failures: &[BenchVerifyFailure],
    implementation: &ImplementationAuthorityBinding,
) -> Gh931AuthorityVerdict {
    let registry = verified.claim_registry.as_ref();
    let policy = registry.map(|artifact| &artifact.value);
    let policy_valid = policy.is_some_and(policy_is_valid);
    let policy_ready = policy_valid && policy.is_some_and(|policy| policy.locked);
    let (report, mut diagnostics) = select_report(verified);
    let runs = report
        .map(|report| report_runs(report, verified, &mut diagnostics))
        .unwrap_or_default();
    let outcomes = report
        .map(|report| coding_outcomes(report, &runs, verified))
        .unwrap_or_default();
    let complete = report.is_some()
        && runs.len() == EXPECTED_RUNS
        && coding_report_structurally_complete(&outcomes);
    let paired_statistics = report
        .map(|_| coding_paired_statistics(&outcomes, verifier_failures.is_empty()))
        .unwrap_or_default();
    let attempts_ready = complete
        && paired_statistics
            .iter()
            .all(|statistic| statistic.status == "computed");
    let machine_outcomes_ready = runs.len() == EXPECTED_RUNS
        && runs
            .iter()
            .all(|run| verified.official_coding_tests.contains_key(&run.path));
    let provenance_ready = runs_are_genuine(&runs, implementation);
    let evidence_ready = complete
        && attempts_ready
        && machine_outcomes_ready
        && verified.official_evidence_authenticated
        && verifier_failures.is_empty()
        && provenance_ready;

    if !complete {
        diagnostics.push("requires one exact complete issue385-v1/official-v1 matrix".to_string());
    } else if !attempts_ready {
        diagnostics.push(
            "official tuples require globally unique nonblank attempt_id and target_started=true"
                .to_string(),
        );
    } else if !machine_outcomes_ready {
        diagnostics.push("official runs lack complete machine-readable test evidence".to_string());
    } else if !verified.official_evidence_authenticated {
        diagnostics.push(
            "official claims require governed scorer and supervisor receipts; repository-local JSON is not authority"
                .to_string(),
        );
    } else if !verifier_failures.is_empty() {
        diagnostics
            .push("benchmark verifier failures make GH931 evidence insufficient".to_string());
    } else if !provenance_ready {
        diagnostics.push(
            "official runs require one exact model identity and the clean current runtime implementation tree"
                .to_string(),
        );
    }
    if policy.is_some_and(|policy| !policy.locked) {
        diagnostics.push("claim registry policy is not locked".to_string());
    }
    if !policy_valid {
        diagnostics.push("claim registry policy is missing or malformed".to_string());
    }

    let stop_loss = evaluate_stop_loss(policy, &runs, evidence_ready, policy_ready);
    let maintenance = evaluate_maintenance(policy, verified, &runs, evidence_ready, policy_ready);
    let claims = evaluate_claims(
        policy,
        &paired_statistics,
        &maintenance,
        &stop_loss,
        evidence_ready,
        policy_ready,
    );
    let status = if claims
        .iter()
        .any(|claim| claim.status == AuthorityStatus::Fail)
    {
        AuthorityStatus::Fail
    } else if !claims.is_empty()
        && claims
            .iter()
            .all(|claim| claim.status == AuthorityStatus::Pass)
    {
        AuthorityStatus::Pass
    } else {
        AuthorityStatus::Insufficient
    };

    Gh931AuthorityVerdict {
        status,
        measurement_ready: evidence_ready,
        registry: registry_binding(registry, policy_valid),
        report: report.map(|report| report_binding(report, &runs)),
        completeness: Gh931Completeness {
            expected_tasks: 16,
            expected_conditions: 3,
            expected_runs_per_task: 3,
            expected_runs: EXPECTED_RUNS,
            observed_runs: runs.len(),
            complete,
            attempts_ready,
            machine_outcomes_ready,
        },
        condition_completion: condition_completion(&runs, verified),
        paired_statistics,
        maintenance,
        stop_loss,
        claims,
        diagnostics,
    }
}

fn condition_completion(
    runs: &[&VerifiedArtifact<super::super::types::CodingRunArtifact>],
    verified: &VerifiedBenchmarkArtifacts,
) -> Vec<Gh931ConditionCompletion> {
    ["no_memory", "remem_e2e", "curated_file_budgeted"]
        .into_iter()
        .map(|condition| {
            let eligible = runs.iter().filter(|run| {
                run.value.condition == condition && run.value.target_started == Some(true)
            });
            let eligible_started = eligible.clone().count();
            let resolved = eligible
                .filter(|run| {
                    verified
                        .official_coding_tests
                        .get(&run.path)
                        .is_some_and(|evidence| evidence.value.resolved())
                })
                .count();
            Gh931ConditionCompletion {
                condition: condition.to_string(),
                eligible_started,
                resolved,
            }
        })
        .collect()
}

fn evaluate_maintenance(
    policy: Option<&ClaimRegistryPolicy>,
    verified: &VerifiedBenchmarkArtifacts,
    runs: &[&VerifiedArtifact<super::super::types::CodingRunArtifact>],
    evidence_ready: bool,
    policy_ready: bool,
) -> Gh931MaintenanceVerdict {
    let curated_runs = runs
        .iter()
        .filter(|run| run.value.condition == "curated_file_budgeted")
        .collect::<Vec<_>>();
    let mut logs_by_task: BTreeMap<
        String,
        &VerifiedArtifact<super::super::types::CuratorLogArtifact>,
    > = BTreeMap::new();
    let mut complete = curated_runs.len() == 48;
    for run in curated_runs {
        let Some(log) = verified.curator_logs.get(&run.path) else {
            complete = false;
            continue;
        };
        if let Some(existing) = logs_by_task.get(&run.value.task_id) {
            if existing.sha256 != log.sha256 {
                complete = false;
            }
        } else {
            logs_by_task.insert(run.value.task_id.clone(), log);
        }
    }
    complete &= logs_by_task.len() == 16;
    let curator_sessions = logs_by_task
        .values()
        .map(|log| log.value.sessions.len())
        .sum::<usize>();
    let curator_minutes = complete.then(|| {
        logs_by_task
            .values()
            .map(|log| log.value.totals.maintenance_minutes)
            .sum::<f64>()
    });
    let curated_minutes_per_100_sessions = curator_minutes.and_then(|minutes| {
        (curator_sessions > 0).then(|| minutes * 100.0 / curator_sessions as f64)
    });
    let treatment_runs = runs
        .iter()
        .filter(|run| run.value.condition == "remem_e2e")
        .collect::<Vec<_>>();
    let treatment_complete = treatment_runs.len() == 48
        && treatment_runs
            .iter()
            .all(|run| verified.treatment_maintenance.contains_key(&run.path));
    let treatment_totals = treatment_complete
        .then(|| {
            treatment_runs
                .iter()
                .filter_map(|run| verified.treatment_maintenance.get(&run.path))
                .try_fold((0.0, 0_usize), |(minutes, sessions), evidence| {
                    let minutes = minutes + evidence.value.minutes();
                    let sessions = sessions.checked_add(evidence.value.session_count())?;
                    (minutes.is_finite() && sessions > 0).then_some((minutes, sessions))
                })
        })
        .flatten();
    let remem_sessions = treatment_totals.map(|(_, sessions)| sessions);
    let remem_minutes_per_100_sessions =
        treatment_totals.map(|(minutes, sessions)| minutes * 100.0 / sessions as f64);
    let reduction_pct = curated_minutes_per_100_sessions
        .zip(remem_minutes_per_100_sessions)
        .and_then(|(curated, remem)| (curated > 0.0).then(|| (curated - remem) * 100.0 / curated));
    let gate = non_inferiority_gate(policy);
    let threshold_failed = gate.is_some_and(|gate| {
        reduction_pct.is_some_and(|reduction| reduction < gate.human_maintenance_reduction_min_pct)
    });
    let status = if evidence_ready && policy_ready && threshold_failed {
        AuthorityStatus::Fail
    } else if evidence_ready
        && policy_ready
        && complete
        && treatment_totals.is_some()
        && reduction_pct.is_some()
        && gate.is_some()
    {
        AuthorityStatus::Pass
    } else {
        AuthorityStatus::Insufficient
    };
    let diagnostics = if evidence_ready && policy_ready && threshold_failed {
        vec!["human-maintenance reduction threshold not satisfied".to_string()]
    } else if status == AuthorityStatus::Insufficient {
        vec!["raw curator or treatment maintenance evidence is incomplete".to_string()]
    } else {
        Vec::new()
    };
    Gh931MaintenanceVerdict {
        status,
        curator_tasks: logs_by_task.len(),
        curator_sessions,
        curator_minutes,
        curated_minutes_per_100_sessions,
        remem_sessions,
        remem_minutes_per_100_sessions,
        reduction_pct,
        diagnostics,
    }
}

fn select_report(
    verified: &VerifiedBenchmarkArtifacts,
) -> (
    Option<&VerifiedArtifact<super::super::types::PublicBenchmarkReport>>,
    Vec<String>,
) {
    let candidates = verified
        .reports
        .iter()
        .filter(|report| {
            let value = &report.value;
            value.benchmark_id == "issue385-v1"
                && value.benchmark_version == "official-v1"
                && value.run_phase.as_deref() == Some("official")
                && value.matrix_namespace.as_deref() == Some("issue385-v1/official-v1")
                && exact_conditions(&value.conditions)
        })
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [report] => (Some(*report), Vec::new()),
        [] => (None, vec!["no exact GH931 official report".to_string()]),
        _ => (
            None,
            vec!["multiple GH931 official reports are ambiguous".to_string()],
        ),
    }
}

fn exact_conditions(conditions: &[String]) -> bool {
    conditions.len() == 3
        && conditions.iter().cloned().collect::<BTreeSet<_>>()
            == BTreeSet::from([
                "no_memory".to_string(),
                "remem_e2e".to_string(),
                "curated_file_budgeted".to_string(),
            ])
}

fn report_runs<'a>(
    report: &VerifiedArtifact<super::super::types::PublicBenchmarkReport>,
    verified: &'a VerifiedBenchmarkArtifacts,
    diagnostics: &mut Vec<String>,
) -> Vec<&'a VerifiedArtifact<super::super::types::CodingRunArtifact>> {
    report
        .value
        .run_artifacts
        .iter()
        .filter_map(|path| {
            let run = verified.coding_runs.iter().find(|run| run.path == *path);
            if run.is_none() {
                diagnostics.push(format!(
                    "report references missing verified coding run {path}"
                ));
            }
            run
        })
        .collect()
}

fn coding_outcomes(
    report: &VerifiedArtifact<super::super::types::PublicBenchmarkReport>,
    runs: &[&VerifiedArtifact<super::super::types::CodingRunArtifact>],
    verified: &VerifiedBenchmarkArtifacts,
) -> Vec<CodingTaskOutcome> {
    runs.iter()
        .map(|artifact| {
            let run = &artifact.value;
            let test_evidence = verified.official_coding_tests.get(&artifact.path);
            let recomputed_resolved =
                test_evidence.is_some_and(|evidence| evidence.value.resolved());
            CodingTaskOutcome {
                report_path: report.path.clone(),
                benchmark_id: run.benchmark_id.clone(),
                benchmark_version: run.benchmark_version.clone(),
                run_phase: run.run_phase.clone(),
                matrix_namespace: run.matrix_namespace.clone(),
                condition: run.condition.clone(),
                task_id: run.task_id.clone(),
                run_index: run.run_index,
                attempt_id: run.attempt_id.clone(),
                target_started: run.target_started,
                resolved: recomputed_resolved,
                failure_reason: test_evidence
                    .and_then(|evidence| evidence.value.failure_reason())
                    .map(str::to_string)
                    .or_else(|| test_evidence.is_none().then(|| "test_failure".to_string())),
                tokens_total: run.metrics.tokens_total,
                turns: run.metrics.turns,
                wall_time_ms: run.metrics.wall_time_ms,
                memory_helped: run
                    .memory_contract
                    .as_ref()
                    .map(|contract| contract.memory_helped),
                memory_hurt: run
                    .memory_contract
                    .as_ref()
                    .map(|contract| contract.memory_hurt),
            }
        })
        .collect()
}

fn runs_are_genuine(
    runs: &[&VerifiedArtifact<super::super::types::CodingRunArtifact>],
    implementation: &ImplementationAuthorityBinding,
) -> bool {
    let mut platforms = BTreeSet::new();
    let mut production_input_trees = BTreeSet::new();
    let model = runs.first().map(|run| &run.value.model);
    let all_runs_bound = !runs.is_empty()
        && runs.iter().all(|run| {
            let environment = &run.value.environment;
            let Some(tree) = environment.production_input_tree_sha256.as_deref() else {
                return false;
            };
            platforms.insert((environment.os.as_str(), environment.arch.as_str()));
            production_input_trees.insert(tree);
            environment.source_dirty == Some(false)
                && super::is_lower_hex(&environment.remem_commit, 40)
                && super::is_lower_hex(tree, 64)
                && !environment.os.trim().is_empty()
                && !environment.arch.trim().is_empty()
                && super::model_identity_is_complete(&run.value.model)
                && model == Some(&run.value.model)
        });
    all_runs_bound
        && platforms.len() == 1
        && production_input_trees.len() == 1
        && super::implementation_allows_release(implementation)
        && implementation
            .checkout_production_input_tree_sha256
            .as_deref()
            .is_some_and(|current| production_input_trees.contains(current))
}

fn evaluate_stop_loss(
    policy: Option<&ClaimRegistryPolicy>,
    runs: &[&VerifiedArtifact<super::super::types::CodingRunArtifact>],
    evidence_ready: bool,
    policy_ready: bool,
) -> Gh931StopLossVerdict {
    let eligible = runs
        .iter()
        .filter(|run| run.value.condition == "remem_e2e")
        .collect::<Vec<_>>();
    let missing_attribution = eligible
        .iter()
        .any(|run| run.value.memory_contract.is_none());
    let memory_hurt_rate_pct = (!eligible.is_empty() && !missing_attribution).then(|| {
        eligible
            .iter()
            .filter(|run| {
                run.value.memory_contract.as_ref().is_some_and(|contract| {
                    super::super::verify::coding::recompute_memory_hurt(
                        run.value.failure_reason.as_deref(),
                        contract.stale_used_count,
                    )
                })
            })
            .count() as f64
            * 100.0
            / eligible.len() as f64
    });
    let stale_memory_followed_rate_pct = (!eligible.is_empty()).then(|| {
        eligible
            .iter()
            .filter(|run| run.value.failure_reason.as_deref() == Some("stale_memory_followed"))
            .count() as f64
            * 100.0
            / eligible.len() as f64
    });
    let gate = stop_loss_gate(policy);
    let breached = gate.is_some_and(|gate| {
        memory_hurt_rate_pct.is_some_and(|rate| rate > gate.memory_hurt_max_pct)
            || stale_memory_followed_rate_pct
                .is_some_and(|rate| rate > gate.stale_memory_followed_max_pct)
    });
    let status = if evidence_ready && policy_ready && breached {
        AuthorityStatus::Fail
    } else if evidence_ready
        && policy_ready
        && !missing_attribution
        && eligible.len() == 48
        && gate.is_some()
    {
        AuthorityStatus::Pass
    } else {
        AuthorityStatus::Insufficient
    };
    let diagnostics = if missing_attribution {
        vec!["one or more remem_e2e runs lack memory_hurt attribution".to_string()]
    } else if evidence_ready && policy_ready && breached {
        vec!["measured GH931 stop-loss threshold breached".to_string()]
    } else if status == AuthorityStatus::Insufficient {
        vec!["stop-loss evidence or locked policy is incomplete".to_string()]
    } else {
        Vec::new()
    };
    Gh931StopLossVerdict {
        status,
        eligible_runs: eligible.len(),
        memory_hurt_rate_pct,
        stale_memory_followed_rate_pct,
        diagnostics,
    }
}

fn evaluate_claims(
    policy: Option<&ClaimRegistryPolicy>,
    statistics: &[CodingPairedStatistic],
    maintenance: &Gh931MaintenanceVerdict,
    stop_loss: &Gh931StopLossVerdict,
    evidence_ready: bool,
    policy_ready: bool,
) -> Vec<Gh931ClaimVerdict> {
    policy
        .into_iter()
        .flat_map(|policy| &policy.claims)
        .map(|claim| {
            let mut diagnostics = Vec::new();
            let status = match claim.id.as_str() {
                NO_MEMORY_CLAIM => superiority_status(
                    claim,
                    statistics,
                    evidence_ready && policy_ready,
                    &mut diagnostics,
                ),
                CURATED_CLAIM => non_inferiority_status(
                    claim,
                    statistics,
                    maintenance,
                    evidence_ready && policy_ready,
                    &mut diagnostics,
                ),
                STOP_LOSS_CLAIM => stop_loss.status,
                _ => AuthorityStatus::Insufficient,
            };
            if claim.status != status {
                diagnostics.push(format!(
                    "declared registry status {:?} differs from recomputed {:?}",
                    claim.status, status
                ));
            }
            Gh931ClaimVerdict {
                id: claim.id.clone(),
                status,
                declared_registry_status: claim.status,
                treatment: claim.comparison.treatment.clone(),
                control: claim.comparison.control.clone(),
                metric: claim.metric.clone(),
                allowed_wording: claim.allowed_wording.clone(),
                forbidden_wording: claim.forbidden_wording.clone(),
                diagnostics,
            }
        })
        .collect()
}

fn superiority_status(
    claim: &ClaimRegistryClaimPolicy,
    statistics: &[CodingPairedStatistic],
    ready: bool,
    diagnostics: &mut Vec<String>,
) -> AuthorityStatus {
    let (ClaimRegistryGate::Superiority(gate), Some(statistic)) = (
        &claim.gate,
        statistics
            .iter()
            .find(|statistic| statistic.comparison_id == claim.id),
    ) else {
        return AuthorityStatus::Insufficient;
    };
    if !ready || statistic.status != "computed" {
        return AuthorityStatus::Insufficient;
    }
    let passed = statistic
        .effect_pp
        .is_some_and(|effect| effect >= gate.min_effect_pp)
        && statistic
            .ci_lower_pp
            .is_some_and(|lower| lower > gate.ci_lower_bound_pp_gt);
    if !passed {
        diagnostics.push("resolved-rate superiority threshold not satisfied".to_string());
    }
    if passed {
        AuthorityStatus::Pass
    } else {
        AuthorityStatus::Fail
    }
}

fn non_inferiority_status(
    claim: &ClaimRegistryClaimPolicy,
    statistics: &[CodingPairedStatistic],
    maintenance: &Gh931MaintenanceVerdict,
    ready: bool,
    diagnostics: &mut Vec<String>,
) -> AuthorityStatus {
    let (ClaimRegistryGate::NonInferiority(gate), Some(statistic)) = (
        &claim.gate,
        statistics
            .iter()
            .find(|statistic| statistic.comparison_id == claim.id),
    ) else {
        return AuthorityStatus::Insufficient;
    };
    if !ready || statistic.status != "computed" {
        return AuthorityStatus::Insufficient;
    }
    let non_inferior =
        statistic
            .effect_pp
            .zip(statistic.ci_lower_pp)
            .is_some_and(|(effect, ci_lower)| {
                meets_non_inferiority_margin(effect, ci_lower, gate.non_inferiority_margin_pp)
            });
    if !non_inferior {
        diagnostics.push("resolved-rate non-inferiority threshold not satisfied".to_string());
        AuthorityStatus::Fail
    } else if maintenance.status == AuthorityStatus::Pass {
        AuthorityStatus::Pass
    } else {
        diagnostics.extend(maintenance.diagnostics.iter().cloned());
        maintenance.status
    }
}

pub(in crate::eval::bench_artifact) fn meets_non_inferiority_margin(
    effect_pp: f64,
    ci_lower_pp: f64,
    margin_pp: f64,
) -> bool {
    let threshold = -margin_pp;
    effect_pp >= threshold && ci_lower_pp >= threshold
}

fn policy_is_valid(policy: &ClaimRegistryPolicy) -> bool {
    if policy.schema_version != 1
        || policy.issue != "#931"
        || policy.claims.len() != 3
        || !registered_wording_matches(policy)
    {
        return false;
    }
    let claims = policy
        .claims
        .iter()
        .map(|claim| (claim.id.as_str(), claim))
        .collect::<BTreeMap<_, _>>();
    claims.len() == 3
        && valid_superiority(claims.get(NO_MEMORY_CLAIM).copied())
        && valid_non_inferiority(claims.get(CURATED_CLAIM).copied())
        && valid_stop_loss(claims.get(STOP_LOSS_CLAIM).copied())
}

fn registered_wording_matches(policy: &ClaimRegistryPolicy) -> bool {
    let wording = policy
        .claims
        .iter()
        .map(|claim| (&claim.id, &claim.allowed_wording, &claim.forbidden_wording))
        .collect::<Vec<_>>();
    serde_json::to_vec(&wording)
        .is_ok_and(|bytes| format!("{:x}", Sha256::digest(bytes)) == REGISTERED_WORDING_SHA256)
}

fn valid_superiority(claim: Option<&ClaimRegistryClaimPolicy>) -> bool {
    claim.is_some_and(|claim| {
        claim.metric == "resolved_rate"
            && claim.comparison.treatment == "remem_e2e"
            && claim.comparison.control == "no_memory"
            && wording_policy_present(claim)
            && matches!(
                &claim.gate,
                ClaimRegistryGate::Superiority(gate) if valid_statistical_gate(
                    gate.ci_level,
                    &gate.statistical_unit,
                    &gate.method,
                ) && gate.min_effect_pp == 10.0
                    && gate.ci_lower_bound_pp_gt == 0.0
            )
    })
}

fn valid_non_inferiority(claim: Option<&ClaimRegistryClaimPolicy>) -> bool {
    claim.is_some_and(|claim| {
        claim.metric == "resolved_rate"
            && claim.comparison.treatment == "remem_e2e"
            && claim.comparison.control == "curated_file_budgeted"
            && wording_policy_present(claim)
            && matches!(
                &claim.gate,
                ClaimRegistryGate::NonInferiority(gate) if valid_statistical_gate(
                    gate.ci_level,
                    &gate.statistical_unit,
                    &gate.method,
                ) && gate.non_inferiority_margin_pp == 3.0
                    && gate.human_maintenance_reduction_min_pct == 70.0
            )
    })
}

fn valid_stop_loss(claim: Option<&ClaimRegistryClaimPolicy>) -> bool {
    claim.is_some_and(|claim| {
        claim.metric == "memory_harm"
            && claim.comparison.treatment == "remem_e2e"
            && claim.comparison.control == "no_memory"
            && wording_policy_present(claim)
            && matches!(
                &claim.gate,
                ClaimRegistryGate::StopLoss(gate) if gate.memory_hurt_max_pct == 2.0
                    && gate.stale_memory_followed_max_pct == 1.0
            )
    })
}

fn wording_policy_present(claim: &ClaimRegistryClaimPolicy) -> bool {
    !claim.allowed_wording.is_empty()
        && !claim.forbidden_wording.is_empty()
        && (claim.supporting_report.is_null() || claim.supporting_report.is_object())
}

fn valid_statistical_gate(ci_level: f64, unit: &str, method: &str) -> bool {
    (ci_level - 0.95).abs() <= f64::EPSILON
        && unit == "task"
        && method == "task-cluster paired bootstrap"
}

fn stop_loss_gate(policy: Option<&ClaimRegistryPolicy>) -> Option<&ClaimStopLossGate> {
    let claim = policy?
        .claims
        .iter()
        .find(|claim| claim.id == STOP_LOSS_CLAIM)?;
    match &claim.gate {
        ClaimRegistryGate::StopLoss(gate) => Some(gate),
        _ => None,
    }
}

fn non_inferiority_gate(
    policy: Option<&ClaimRegistryPolicy>,
) -> Option<&super::super::types::ClaimNonInferiorityGate> {
    let claim = policy?
        .claims
        .iter()
        .find(|claim| claim.id == CURATED_CLAIM)?;
    match &claim.gate {
        ClaimRegistryGate::NonInferiority(gate) => Some(gate),
        _ => None,
    }
}

fn registry_binding(
    registry: Option<&VerifiedArtifact<ClaimRegistryPolicy>>,
    policy_valid: bool,
) -> Gh931RegistryBinding {
    let policy = registry.map(|artifact| &artifact.value);
    Gh931RegistryBinding {
        path: registry.map(|artifact| artifact.path.clone()),
        sha256: registry.map(|artifact| artifact.sha256.clone()),
        schema_version: policy.map(|policy| policy.schema_version),
        issue: policy.map(|policy| policy.issue.clone()),
        locked: policy.is_some_and(|policy| policy.locked),
        policy_valid,
        declared_statuses: policy
            .into_iter()
            .flat_map(|policy| &policy.claims)
            .map(|claim| claim.status)
            .collect(),
    }
}

fn report_binding(
    report: &VerifiedArtifact<super::super::types::PublicBenchmarkReport>,
    runs: &[&VerifiedArtifact<super::super::types::CodingRunArtifact>],
) -> Gh931ReportBinding {
    let mut models_by_condition: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    let mut platforms = BTreeSet::new();
    let mut producing_shas = BTreeSet::new();
    let mut production_input_trees = BTreeSet::new();
    let mut source_dirty_attestations = BTreeSet::new();
    for run in runs {
        let model_key = serde_json::to_string(&run.value.model).unwrap_or_default();
        models_by_condition
            .entry(run.value.condition.clone())
            .or_default()
            .insert(model_key, run.value.model.clone());
        platforms.insert(format!(
            "{}/{}",
            run.value.environment.os, run.value.environment.arch
        ));
        producing_shas.insert(run.value.environment.remem_commit.clone());
        if let Some(tree) = &run.value.environment.production_input_tree_sha256 {
            production_input_trees.insert(tree.clone());
        }
        source_dirty_attestations.insert(run.value.environment.source_dirty);
    }
    Gh931ReportBinding {
        path: report.path.clone(),
        sha256: report.sha256.clone(),
        conditions: report.value.conditions.clone(),
        models_by_condition: models_by_condition
            .into_iter()
            .map(|(condition, models)| (condition, models.into_values().collect()))
            .collect(),
        platforms: platforms.into_iter().collect(),
        producing_shas: producing_shas.into_iter().collect(),
        production_input_trees: production_input_trees.into_iter().collect(),
        source_dirty_attestations: source_dirty_attestations.into_iter().collect(),
    }
}
