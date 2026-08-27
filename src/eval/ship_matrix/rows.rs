use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    evidence_for_path, public_model_identity, read_json_value, ArtifactState,
    ImplementationIdentity, PublicEvidence, ShipGateRow, ShipGateStatus, ShipMatrixOptions,
};
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
    implementation: &ImplementationIdentity,
) -> Vec<ShipGateRow> {
    let deterministic = component_gate(
        "deterministic_retrieval",
        "src/eval/gates.rs",
        &["golden."],
        GOLDEN_METRIC_SET_SHA256,
        deltas,
        vec![
            evidence_for_path(
                Path::new(&options.golden_dataset_path),
                ArtifactState::Verified,
            ),
            evidence_for_path(Path::new(&options.baseline_path), ArtifactState::Verified),
            evidence_for_path(Path::new(&options.thresholds_path), ArtifactState::Verified),
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
            vec![evidence_for_path(
                Path::new(&options.golden_dataset_path),
                ArtifactState::Verified,
            )],
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
            vec![evidence_for_path(
                Path::new(&options.baseline_path),
                ArtifactState::Verified,
            )],
            vec!["merge", "release", "context_default"],
        ),
        security_gate(options, public),
        cross_host_gate(options),
        coding_gate(options, public, implementation),
        public_claim_gate(options, public, implementation),
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
        metric_deltas: BTreeMap::new(),
        stop_loss_verdict: "incomplete_missing_capacity_evidence".to_string(),
        exclusions: Vec::new(),
        evidence: vec![evidence_for_path(
            Path::new(&options.golden_dataset_path),
            ArtifactState::Verified,
        )],
        diagnostics: vec![
            "Capacity evidence is required; a dataset without a fixture corpus cannot pass the gate."
                .to_string(),
        ],
    }
}

fn security_gate(options: &ShipMatrixOptions, public: &PublicEvidence) -> ShipGateRow {
    let status = if public.security_authority.passed && public.security.is_some() {
        ShipGateStatus::Pass
    } else {
        ShipGateStatus::Fail
    };
    let mut diagnostics = Vec::new();
    diagnostics.extend(public.report_error.iter().cloned());
    diagnostics.extend(public.security_error.iter().cloned());
    diagnostics.extend(public.security_authority.diagnostics.iter().cloned());
    ShipGateRow {
        id: "production_security_e2e",
        owner: "src/eval/memory_bench + eval/public/memory",
        status,
        blocks: vec!["merge", "release", "security_claim"],
        required_for_command_success: true,
        claim_level: public
            .security
            .as_ref()
            .and_then(|value| value.get("claim_level"))
            .and_then(Value::as_str)
            .unwrap_or("unavailable_no_public_claim")
            .to_string(),
        condition_completeness: if public.security_authority.passed {
            format!(
                "production-path adversarial-policy v2 is manifest- and source-bound to {}",
                public
                    .security_authority
                    .benchmark_commit
                    .as_deref()
                    .unwrap_or("unavailable")
            )
        } else {
            "missing verified production-path adversarial-policy v2".to_string()
        },
        config_identity: "public benchmark manifest and schema verifier".to_string(),
        model_identity: public_model_identity(public),
        metric_deltas: BTreeMap::new(),
        stop_loss_verdict: if status == ShipGateStatus::Pass {
            "passed_production_security_stop_loss_and_identity"
        } else {
            "failed_or_incomplete_security_evidence"
        }
        .to_string(),
        exclusions: Vec::new(),
        evidence: vec![evidence_for_path(
            &options.security_report_path,
            if status == ShipGateStatus::Pass {
                ArtifactState::Verified
            } else {
                ArtifactState::Invalid
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

fn coding_gate(
    options: &ShipMatrixOptions,
    public: &PublicEvidence,
    implementation: &ImplementationIdentity,
) -> ShipGateRow {
    let artifacts_verified = public
        .report
        .as_ref()
        .is_some_and(|report| report.artifact_verifier.passed);
    let passed = artifacts_verified && public.claim_authority.coding_passed;
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
        condition_completeness: if public.claim_authority.coding_passed {
            "all registered coding claims are locked, PASS, and hash-bound"
        } else {
            "registered coding claim authority is incomplete"
        }
        .to_string(),
        config_identity: "GH931 registered 16 x 3 x 3 official matrix".to_string(),
        model_identity: public_model_identity(public),
        metric_deltas: BTreeMap::new(),
        stop_loss_verdict: if passed { "passed" } else { "unavailable" }.to_string(),
        exclusions: vec!["smoke_or_unregistered_coding_runs".to_string()],
        evidence: vec![
            evidence_for_path(
                &options.public_root.join("reports/baseline.json"),
                if public.report.is_some() {
                    ArtifactState::Verified
                } else {
                    ArtifactState::Invalid
                },
            ),
            evidence_for_path(
                &options.claim_registry_path,
                if public.claim_authority.coding_passed {
                    ArtifactState::Verified
                } else {
                    ArtifactState::Present
                },
            ),
        ],
        diagnostics: claim_diagnostics(
            public,
            implementation,
            Some(&public.claim_authority.diagnostics),
        ),
    }
}

fn public_claim_gate(
    options: &ShipMatrixOptions,
    public: &PublicEvidence,
    implementation: &ImplementationIdentity,
) -> ShipGateRow {
    let passed = public.claim_authority.level3_passed;
    ShipGateRow {
        id: "public_claim",
        owner: "eval/claims + scripts/ci/check_public_claims.py",
        status: if passed {
            ShipGateStatus::Pass
        } else {
            ShipGateStatus::Unavailable
        },
        blocks: vec!["public_claim"],
        required_for_command_success: false,
        claim_level: if passed {
            "level_3_independently_verified_public_claim"
        } else {
            "unavailable_no_level_3_claim_authority"
        }
        .to_string(),
        condition_completeness: if passed {
            "complete independently verified claim artifact"
        } else {
            "no Level 3 public claim artifact"
        }
        .to_string(),
        config_identity: "claim registry and public wording guard".to_string(),
        model_identity: public_model_identity(public),
        metric_deltas: BTreeMap::new(),
        stop_loss_verdict: if passed { "passed" } else { "unavailable" }.to_string(),
        exclusions: vec!["non_independently_verified_claim_evidence".to_string()],
        evidence: vec![evidence_for_path(
            &options.claim_registry_path,
            if options.claim_registry_path.is_file() {
                ArtifactState::Present
            } else {
                ArtifactState::Missing
            },
        )],
        diagnostics: if passed {
            Vec::new()
        } else {
            let mut diagnostics = claim_diagnostics(
                public,
                implementation,
                Some(&public.claim_authority.diagnostics),
            );
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
        metric_deltas: BTreeMap::new(),
        stop_loss_verdict: "unavailable_no_default_decision".to_string(),
        exclusions: vec!["regression_only_evidence_without_capability_ablation".to_string()],
        evidence: Vec::new(),
        diagnostics: vec![diagnostic.to_string()],
    }
}

fn claim_diagnostics(
    public: &PublicEvidence,
    implementation: &ImplementationIdentity,
    notes: Option<&[String]>,
) -> Vec<String> {
    let mut diagnostics = notes.map(<[String]>::to_vec).unwrap_or_else(|| {
        vec![public
            .report_error
            .clone()
            .unwrap_or_else(|| "public baseline unavailable".to_string())]
    });
    if !public.claim_authority.coding_passed {
        diagnostics.push(format!(
            "each required claim supporting report must bind current implementation SHA {}",
            implementation.git_sha.as_deref().unwrap_or("unavailable")
        ));
    }
    diagnostics
}
