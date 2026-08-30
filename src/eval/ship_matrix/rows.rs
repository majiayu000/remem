use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    evidence_for_evaluated_path, evidence_for_path, read_json_value, ArtifactEvidence,
    ArtifactState, PublicEvidence, ShipGateRow, ShipGateStatus, ShipMatrixOptions,
};
use crate::eval::bench_artifact::AuthorityStatus;
use crate::eval::gates::{EvalGateDelta, EvalGateStatus};

const GOLDEN_METRIC_SET_SHA256: &str =
    "d6fc5c39586c3665293bfcca366491f5c1bb93444d3adff3d84b311b4a347611";
const CAPACITY_METRIC_SET_SHA256: &str =
    "cb83441e26b37a06803b6eadbb3dd5592b9b9ab3d8307d8ed00b3296f52bb038";
const SESSION_METRIC_SET_SHA256: &str =
    "f97ebbf86b72e58327683405cd0df4c078607cfad4e7aa65815f5b78e21c0926";

pub(super) fn build_gate_rows(
    deltas: &[EvalGateDelta],
    capacity_applicable: bool,
    options: &ShipMatrixOptions,
    public: &PublicEvidence,
) -> Vec<ShipGateRow> {
    let deterministic = component_gate(
        "deterministic_retrieval",
        "src/eval/gates.rs",
        &["golden."],
        GOLDEN_METRIC_SET_SHA256,
        deltas,
        vec![
            evaluated_evidence(options, &options.golden_dataset_path),
            evaluated_evidence(options, &options.baseline_path),
            evaluated_evidence(options, &options.thresholds_path),
        ],
        vec!["merge", "release", "retrieval_default"],
    );
    let capacity = if capacity_applicable {
        component_gate(
            "capacity",
            "src/eval/capacity.rs",
            &["capacity."],
            CAPACITY_METRIC_SET_SHA256,
            deltas,
            vec![evaluated_evidence(options, &options.golden_dataset_path)],
            vec!["merge", "release", "retrieval_default"],
        )
    } else {
        incomplete_capacity(options)
    };
    vec![
        deterministic,
        capacity,
        component_gate(
            "session_start",
            "src/eval/injection.rs + src/eval/current_memory_contracts.rs",
            &["injection.", "current_memory_contracts."],
            SESSION_METRIC_SET_SHA256,
            deltas,
            vec![evaluated_evidence(options, &options.baseline_path)],
            vec!["merge", "release", "context_default"],
        ),
        security_gate(options, public),
        cross_host_gate(options),
        coding_gate(options, public),
        public_claim_gate(options, public),
        default_decision_gate(
            "retrieval_default_decision",
            "retrieval_default",
            "docs/specs/GH934 + eval/provider-comparison",
            "No accepted same-head baseline/enhanced retrieval ablation and rollback decision artifact exists.",
        ),
        default_decision_gate(
            "context_default_decision",
            "context_default",
            "docs/specs/GH932 + eval/injection",
            "No accepted capability-specific Context Bundle default decision artifact exists.",
        ),
    ]
}

fn evaluated_evidence(options: &ShipMatrixOptions, path: &str) -> super::ArtifactEvidence {
    evidence_for_evaluated_path(
        Path::new(path),
        options.input_artifact_sha256.get(path).map(String::as_str),
    )
}

pub(super) fn component_gate(
    id: &'static str,
    owner: &'static str,
    prefixes: &[&str],
    expected_metric_set_sha256: &str,
    deltas: &[EvalGateDelta],
    evidence: Vec<super::ArtifactEvidence>,
    blocks: Vec<&'static str>,
) -> ShipGateRow {
    let relevant = deltas
        .iter()
        .filter(|delta| {
            prefixes
                .iter()
                .any(|prefix| delta.metric.starts_with(prefix))
        })
        .collect::<Vec<_>>();
    let evidence_complete = evidence
        .iter()
        .all(|item| item.state == ArtifactState::Verified);
    let actual_metric_set_sha256 = metric_set_sha256(&relevant);
    let metric_set_complete = actual_metric_set_sha256 == expected_metric_set_sha256;
    let status = if relevant.is_empty() || !metric_set_complete {
        ShipGateStatus::Incomplete
    } else if !evidence_complete {
        ShipGateStatus::Fail
    } else if relevant
        .iter()
        .all(|delta| delta.status == EvalGateStatus::Pass)
    {
        ShipGateStatus::Pass
    } else {
        ShipGateStatus::Fail
    };
    let mut diagnostics = relevant
        .iter()
        .filter(|delta| delta.status != EvalGateStatus::Pass)
        .map(|delta| format!("{} is {}", delta.metric, delta.status.label()))
        .collect::<Vec<_>>();
    diagnostics.extend(
        evidence
            .iter()
            .filter(|item| item.state != ArtifactState::Verified)
            .map(|item| format!("required evidence is {:?}: {}", item.state, item.path)),
    );
    if !metric_set_complete {
        diagnostics.push(format!(
            "metric set identity mismatch: expected {expected_metric_set_sha256}, got {actual_metric_set_sha256}"
        ));
    }
    ShipGateRow {
        id,
        owner,
        status,
        blocks,
        required_for_command_success: true,
        claim_level: "component_gate_only".to_string(),
        condition_completeness: if relevant.is_empty() {
            "no matching metrics".to_string()
        } else if !metric_set_complete {
            format!("incomplete metric set; {} matching metrics", relevant.len())
        } else {
            format!("{} checked metrics", relevant.len())
        },
        config_identity: "eval-gates baseline and thresholds artifact hashes".to_string(),
        model_identity: "none_deterministic".to_string(),
        platform_identity: "none_deterministic".to_string(),
        metric_deltas: relevant
            .iter()
            .map(|delta| (delta.metric.clone(), delta.delta))
            .collect(),
        stop_loss_verdict: match status {
            ShipGateStatus::Pass => "passed",
            ShipGateStatus::Fail => "failed",
            _ => "incomplete",
        }
        .to_string(),
        exclusions: Vec::new(),
        evidence,
        diagnostics,
    }
}

fn metric_set_sha256(relevant: &[&EvalGateDelta]) -> String {
    let mut names = relevant
        .iter()
        .map(|delta| delta.metric.as_str())
        .collect::<Vec<_>>();
    names.sort_unstable();
    let mut hasher = Sha256::new();
    for name in names {
        hasher.update(name.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

fn incomplete_capacity(options: &ShipMatrixOptions) -> ShipGateRow {
    ShipGateRow {
        id: "capacity",
        owner: "src/eval/capacity.rs",
        status: ShipGateStatus::Incomplete,
        blocks: vec!["merge", "release", "retrieval_default"],
        required_for_command_success: true,
        claim_level: "component_gate_only".to_string(),
        condition_completeness: "dataset has no fixture corpus".to_string(),
        config_identity: "eval-gates checked-in thresholds".to_string(),
        model_identity: "none_deterministic".to_string(),
        platform_identity: "none_deterministic".to_string(),
        metric_deltas: BTreeMap::new(),
        stop_loss_verdict: "incomplete_missing_capacity_evidence".to_string(),
        exclusions: Vec::new(),
        evidence: vec![evaluated_evidence(options, &options.golden_dataset_path)],
        diagnostics: vec![
            "Capacity evidence is required; a dataset without a fixture corpus cannot pass the gate."
                .to_string(),
        ],
    }
}

fn security_gate(options: &ShipMatrixOptions, public: &PublicEvidence) -> ShipGateRow {
    let verifier_passed = public
        .report
        .as_ref()
        .is_some_and(|report| report.artifact_verifier.passed);
    let authority = public.security_authority.as_ref();
    let current_implementation = public.authority_verdict().is_some_and(|verdict| {
        let implementation = &verdict.implementation;
        implementation.executable_source_equivalent
            && implementation.checkout_source_dirty == Some(false)
            && implementation
                .checkout_production_input_tree_sha256
                .is_some()
            && authority.is_some_and(|authority| {
                authority.production_input_trees.len() == 1
                    && authority.production_input_trees.first()
                        == implementation
                            .checkout_production_input_tree_sha256
                            .as_ref()
            })
    });
    let status = match authority.map(|authority| authority.status) {
        Some(AuthorityStatus::Pass) if verifier_passed && current_implementation => {
            ShipGateStatus::Pass
        }
        Some(AuthorityStatus::Fail) => ShipGateStatus::Fail,
        Some(AuthorityStatus::Insufficient) | Some(AuthorityStatus::Pass) | None => {
            ShipGateStatus::Incomplete
        }
    };
    let mut diagnostics = Vec::new();
    diagnostics.extend(public.report_error.iter().cloned());
    diagnostics.extend(public.security_error.iter().cloned());
    diagnostics.extend(
        authority
            .into_iter()
            .flat_map(|authority| authority.diagnostics.iter().cloned()),
    );
    if !verifier_passed {
        diagnostics.push("public artifact verifier did not pass".to_string());
    }
    if !current_implementation {
        diagnostics.push(
            "security report is not bound to the current verifier implementation tree".to_string(),
        );
    }
    let metric_deltas = authority
        .and_then(|authority| authority.policy_summary.as_ref())
        .map(|summary| {
            BTreeMap::from([
                (
                    "non_retention_leak_rate".to_string(),
                    summary.non_retention_leak_rate,
                ),
                ("false_block_rate".to_string(), summary.false_block_rate),
                (
                    "suppression_obeyed_rate".to_string(),
                    summary.suppression_obeyed_rate,
                ),
                (
                    "policy_failure_rate".to_string(),
                    summary.policy_failure_rate,
                ),
            ])
        })
        .unwrap_or_default();
    ShipGateRow {
        id: "production_security_e2e",
        owner: "src/eval/memory_bench + eval/public/memory",
        status,
        blocks: vec!["merge", "release", "security_claim"],
        required_for_command_success: true,
        claim_level: public
            .security_claim_level
            .clone()
            .unwrap_or_else(|| "unavailable_no_public_claim".to_string()),
        condition_completeness: if status == ShipGateStatus::Pass {
            format!(
                "{} recomputed runs for {}",
                authority.map_or(0, |authority| authority.runs_recomputed),
                authority
                    .and_then(|authority| authority.target.as_deref())
                    .unwrap_or("unavailable target")
            )
        } else {
            "missing verified production-path adversarial-policy v2".to_string()
        },
        config_identity: authority
            .map(|authority| format!("report_sha256={}", authority.report_sha256))
            .unwrap_or_else(|| "unavailable".to_string()),
        model_identity: authority
            .map(|authority| model_identity(&authority.models))
            .unwrap_or_else(|| "unavailable".to_string()),
        platform_identity: authority
            .and_then(|authority| authority.target.clone())
            .unwrap_or_else(|| "unavailable".to_string()),
        metric_deltas,
        stop_loss_verdict: if status == ShipGateStatus::Pass {
            "passed_production_security_stop_loss_and_identity"
        } else {
            "failed_or_incomplete_security_evidence"
        }
        .to_string(),
        exclusions: Vec::new(),
        evidence: vec![authority.map_or_else(
            || ArtifactEvidence {
                path: options.security_report_path.to_string_lossy().to_string(),
                state: ArtifactState::Missing,
                sha256: None,
                detail: "no exact runtime authority binding".to_string(),
            },
            |authority| ArtifactEvidence {
                path: options
                    .public_root
                    .join(&authority.report_path)
                    .to_string_lossy()
                    .to_string(),
                state: if status == ShipGateStatus::Pass {
                    ArtifactState::Verified
                } else {
                    ArtifactState::Invalid
                },
                sha256: Some(authority.report_sha256.clone()),
                detail: "exact report bytes consumed by benchmark verifier".to_string(),
            },
        )],
        diagnostics,
    }
}

fn cross_host_gate(options: &ShipMatrixOptions) -> ShipGateRow {
    let charter = read_json_value(&options.cross_host_charter_path);
    ShipGateRow {
        id: "cross_host",
        owner: "docs/specs/GH935 + eval/cross-host",
        status: ShipGateStatus::Unavailable,
        blocks: vec!["cross_host", "public_claim"],
        required_for_command_success: false,
        claim_level: "infrastructure_only_no_cross_host_claim".to_string(),
        condition_completeness: charter
            .as_ref()
            .ok()
            .and_then(|value| value.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("missing_or_invalid_charter")
            .to_string(),
        config_identity: "GH935 sealed matrix charter".to_string(),
        model_identity: "unavailable_until_governed_runs".to_string(),
        platform_identity: "unavailable_until_governed_runs".to_string(),
        metric_deltas: BTreeMap::new(),
        stop_loss_verdict: "unavailable_no_verified_sealed_result".to_string(),
        exclusions: vec!["no_verified_cross_host_execution".to_string()],
        evidence: vec![evidence_for_path(
            &options.cross_host_charter_path,
            if charter.is_ok() {
                ArtifactState::Present
            } else {
                ArtifactState::Invalid
            },
        )],
        diagnostics: vec![
            "The charter is configuration, not a sealed execution result; GH935 has no result verifier yet."
                .to_string(),
        ],
    }
}

pub(super) fn coding_gate(options: &ShipMatrixOptions, public: &PublicEvidence) -> ShipGateRow {
    let artifacts_verified = public
        .report
        .as_ref()
        .is_some_and(|report| report.artifact_verifier.passed);
    let authority = public.authority_verdict().map(|verdict| &verdict.gh931);
    let passed = artifacts_verified
        && authority.is_some_and(|authority| authority.status == AuthorityStatus::Pass);
    let report = authority.and_then(|authority| authority.report.as_ref());
    ShipGateRow {
        id: "coding_outcome",
        owner: "docs/specs/GH931 + src/eval/bench_artifact",
        status: if passed {
            ShipGateStatus::Pass
        } else {
            ShipGateStatus::Unavailable
        },
        blocks: vec!["coding_outcome", "public_claim"],
        required_for_command_success: false,
        claim_level: if passed {
            "level_2_registered_coding_outcome_claim"
        } else {
            "unavailable_no_authorized_coding_claim"
        }
        .to_string(),
        condition_completeness: authority
            .map(|authority| {
                format!(
                    "{}/{} official runs; complete={}; attempts_ready={}",
                    authority.completeness.observed_runs,
                    authority.completeness.expected_runs,
                    authority.completeness.complete,
                    authority.completeness.attempts_ready
                )
            })
            .unwrap_or_else(|| "runtime GH931 authority unavailable".to_string()),
        config_identity: "GH931 registered 16 x 3 x 3 official matrix".to_string(),
        model_identity: report
            .map(coding_report_model_identity)
            .unwrap_or_else(|| "unavailable".to_string()),
        platform_identity: report
            .map(|report| report.platforms.join(","))
            .filter(|identity| !identity.is_empty())
            .unwrap_or_else(|| "unavailable".to_string()),
        metric_deltas: authority
            .map(|authority| paired_metrics(&authority.paired_statistics))
            .unwrap_or_default(),
        stop_loss_verdict: authority
            .map(|authority| authority_status(authority.stop_loss.status))
            .unwrap_or("unavailable")
            .to_string(),
        exclusions: vec!["smoke_or_unregistered_coding_runs".to_string()],
        evidence: coding_evidence(options, authority, passed),
        diagnostics: claim_diagnostics(public),
    }
}

fn public_claim_gate(options: &ShipMatrixOptions, public: &PublicEvidence) -> ShipGateRow {
    let authority = public.authority_verdict().map(|verdict| &verdict.gh931);
    let report = authority.and_then(|authority| authority.report.as_ref());
    ShipGateRow {
        id: "public_claim",
        owner: "eval/claims + scripts/ci/check_public_claims.py",
        status: ShipGateStatus::Unavailable,
        blocks: vec!["public_claim"],
        required_for_command_success: false,
        claim_level: "unavailable_no_level_3_claim_authority".to_string(),
        condition_completeness: "no independently verified Level 3 authority modeled".to_string(),
        config_identity: "claim registry and public wording guard".to_string(),
        model_identity: report
            .map(coding_report_model_identity)
            .unwrap_or_else(|| "unavailable".to_string()),
        platform_identity: report
            .map(|report| report.platforms.join(","))
            .filter(|identity| !identity.is_empty())
            .unwrap_or_else(|| "unavailable".to_string()),
        metric_deltas: BTreeMap::new(),
        stop_loss_verdict: "unavailable".to_string(),
        exclusions: vec!["non_independently_verified_claim_evidence".to_string()],
        evidence: coding_evidence(options, authority, false),
        diagnostics: {
            let mut diagnostics = claim_diagnostics(public);
            diagnostics
                .push("Comparative, superiority, and SOTA wording remains blocked.".to_string());
            diagnostics
        },
    }
}

fn default_decision_gate(
    id: &'static str,
    scope: &'static str,
    owner: &'static str,
    diagnostic: &'static str,
) -> ShipGateRow {
    ShipGateRow {
        id,
        owner,
        status: ShipGateStatus::Unavailable,
        blocks: vec![scope],
        required_for_command_success: false,
        claim_level: "no_default_on_claim".to_string(),
        condition_completeness: "missing accepted capability-specific decision artifact"
            .to_string(),
        config_identity:
            "same-head baseline/enhanced ablation, thresholds, latency budget, and rollback"
                .to_string(),
        model_identity: "must be declared by decision artifact".to_string(),
        platform_identity: "must be declared by decision artifact".to_string(),
        metric_deltas: BTreeMap::new(),
        stop_loss_verdict: "unavailable_no_default_decision".to_string(),
        exclusions: vec!["regression_only_evidence_without_capability_ablation".to_string()],
        evidence: Vec::new(),
        diagnostics: vec![diagnostic.to_string()],
    }
}

fn claim_diagnostics(public: &PublicEvidence) -> Vec<String> {
    let Some(authority) = public.authority_verdict().map(|verdict| &verdict.gh931) else {
        return vec![public
            .report_error
            .clone()
            .unwrap_or_else(|| "runtime GH931 authority unavailable".to_string())];
    };
    let mut diagnostics = authority.diagnostics.clone();
    diagnostics.extend(authority.maintenance.diagnostics.iter().cloned());
    diagnostics.extend(authority.stop_loss.diagnostics.iter().cloned());
    diagnostics.extend(
        authority
            .claims
            .iter()
            .flat_map(|claim| claim.diagnostics.iter().cloned()),
    );
    diagnostics
}

fn model_identity(models: &[Value]) -> String {
    let identities = models
        .iter()
        .filter_map(|model| serde_json::to_string(model).ok())
        .collect::<Vec<_>>();
    if identities.is_empty() {
        "unavailable".to_string()
    } else {
        identities.join(",")
    }
}

fn coding_report_model_identity(
    report: &crate::eval::bench_artifact::Gh931ReportBinding,
) -> String {
    report
        .models_by_condition
        .iter()
        .map(|(condition, models)| format!("{condition}={}", model_identity(models)))
        .collect::<Vec<_>>()
        .join(";")
}

fn paired_metrics(
    statistics: &[crate::eval::bench_artifact::CodingPairedStatistic],
) -> BTreeMap<String, f64> {
    let mut metrics = BTreeMap::new();
    for statistic in statistics {
        if let Some(effect) = statistic.effect_pp {
            metrics.insert(format!("{}.effect_pp", statistic.comparison_id), effect);
        }
        if let Some(lower) = statistic.ci_lower_pp {
            metrics.insert(format!("{}.ci_lower_pp", statistic.comparison_id), lower);
        }
    }
    metrics
}

fn authority_status(status: AuthorityStatus) -> &'static str {
    match status {
        AuthorityStatus::Pass => "passed",
        AuthorityStatus::Fail => "failed",
        AuthorityStatus::Insufficient => "unavailable",
    }
}

fn coding_evidence(
    options: &ShipMatrixOptions,
    authority: Option<&crate::eval::bench_artifact::Gh931AuthorityVerdict>,
    passed: bool,
) -> Vec<ArtifactEvidence> {
    let mut evidence = Vec::new();
    if let Some(report) = authority.and_then(|authority| authority.report.as_ref()) {
        evidence.push(ArtifactEvidence {
            path: options
                .public_root
                .join(&report.path)
                .to_string_lossy()
                .to_string(),
            state: if passed {
                ArtifactState::Verified
            } else {
                ArtifactState::Present
            },
            sha256: Some(report.sha256.clone()),
            detail: "exact report bytes consumed by benchmark verifier".to_string(),
        });
    }
    if let Some(registry) = authority.map(|authority| &authority.registry) {
        evidence.push(ArtifactEvidence {
            path: registry
                .path
                .clone()
                .unwrap_or_else(|| options.claim_registry_path.to_string_lossy().to_string()),
            state: if registry.policy_valid && registry.locked {
                ArtifactState::Verified
            } else {
                ArtifactState::Invalid
            },
            sha256: registry.sha256.clone(),
            detail:
                "policy bytes consumed by benchmark verifier; declarations are non-authoritative"
                    .to_string(),
        });
    }
    evidence
}
